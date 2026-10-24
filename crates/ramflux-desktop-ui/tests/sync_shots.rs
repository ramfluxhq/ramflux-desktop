// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Offscreen light and dark screenshots of Settings › Diagnostics while the
//! account follows the daemon's event stream, then after the daemon stopped
//! offering it (polling), on a fake daemon (see `sdk_fake`), with the
//! developer panel closed.
//!
//! Not part of `cargo test`; run explicitly:
//!
//! ```sh
//! RAMFLUX_SHOTS_DIR=/some/dir cargo test -p ramflux-desktop-ui --test sync_shots
//! ```

mod common;
mod sdk_fake;

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    shots::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("sync shots: skipped; GPUI has no headless renderer on this platform");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod shots {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use gpui_kit::base::Root;
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Bounds, Entity, HeadlessAppContext, Point, WindowBounds, WindowHandle,
        WindowOptions, px, size,
    };
    use ramflux_desktop_core::SyncMode;
    use ramflux_desktop_ui::AppAssets as Assets;
    use ramflux_desktop_ui::Shell;
    use ramflux_desktop_ui::route::Page;
    use ramflux_desktop_ui::settings::SettingsSection;
    use serde_json::json;

    use crate::sdk_fake;

    const ACCOUNT: &str = "acct-main";

    struct Run {
        cx: HeadlessAppContext,
        handle: WindowHandle<Root>,
        shell: Entity<Shell>,
        out: PathBuf,
    }

    impl Run {
        fn tick(&mut self) {
            std::thread::sleep(Duration::from_millis(10));
            self.cx.advance_clock(Duration::from_millis(50));
            self.cx.run_until_parked();
        }

        fn until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
            let limit = Instant::now() + Duration::from_secs(30);
            while !done(self) {
                assert!(Instant::now() < limit, "timed out waiting for {what}");
                self.tick();
            }
        }

        fn mode(&mut self) -> Option<SyncMode> {
            self.cx.update(|cx| {
                self.shell.read(cx).store().read(cx).store().sync_status().map(|s| s.mode)
            })
        }

        fn render(&mut self) {
            self.cx
                .update_window(self.handle.into(), |_, window, cx| window.render_frame(cx))
                .expect("window");
            self.cx.run_until_parked();
        }

        fn shot(&mut self, name: &str) {
            let dev = self.cx.update(|cx| self.shell.read(cx).is_dev_panel_visible());
            assert!(!dev, "screenshots are taken with the developer panel closed");
            for (mode, suffix) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
                self.cx.update(|cx| {
                    Theme::change(mode, None, cx);
                    cx.refresh_windows();
                });
                self.render();
                let image = self.cx.capture_screenshot(self.handle.into()).expect("render");
                let path = self.out.join(format!("{name}_{suffix}.png"));
                image.save(&path).expect("write png");
                println!("wrote {}", path.display());
            }
            self.cx.update(|cx| {
                Theme::change(ThemeMode::Light, None, cx);
                cx.refresh_windows();
            });
            self.cx.run_until_parked();
        }
    }

    pub fn run() {
        let out = std::env::var_os("RAMFLUX_SHOTS_DIR").map_or_else(
            || PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("sync-shots"),
            PathBuf::from,
        );
        std::fs::create_dir_all(&out).expect("output directory");
        let (fake, _state, socket) = sdk_fake::start("f11-shots");
        fake.enable_stream();
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            ramflux_desktop_ui::init(cx);
        });
        let options = sdk_fake::options(&socket);
        let (handle, shell) = cx
            .update(|cx| {
                let bounds = Bounds { origin: Point::default(), size: size(px(1280.), px(1800.)) };
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        focus: false,
                        show: false,
                        ..Default::default()
                    },
                    cx,
                    |window, cx| cx.new(|cx| Shell::with_options(options, window, cx)),
                )
            })
            .expect("offscreen window");
        let handle = handle.downcast::<Root>().expect("Root");
        let mut run = Run { cx, handle, shell, out };
        run.until("the workspace", |r| {
            r.cx.update(|cx| r.shell.read(cx).page()) == Page::Workspace
        });
        run.until("the stream", |r| r.mode() == Some(SyncMode::Stream));
        fake.publish(
            ACCOUNT,
            "sync.status",
            json!({ "state": "idle", "last_run": 1_790_000_100, "unrouted": 0 }),
        );
        run.until("the daemon's sync", |r| {
            r.cx.update(|cx| {
                r.shell
                    .read(cx)
                    .store()
                    .read(cx)
                    .store()
                    .sync_status()
                    .is_some_and(|s| s.daemon_sync.is_some())
            })
        });
        let shell = run.shell.clone();
        run.cx.update(|cx| {
            shell.update(cx, |s, cx| s.open_settings_at(SettingsSection::Diagnostics, cx));
        });
        run.tick();
        run.shot("f11_01_diagnostics_stream");

        fake.disable_stream();
        fake.end_streams(ACCOUNT, false);
        run.until("polling", |r| r.mode() == Some(SyncMode::Polling));
        run.tick();
        run.shot("f11_02_diagnostics_polling");
        fake.shutdown();
    }
}
