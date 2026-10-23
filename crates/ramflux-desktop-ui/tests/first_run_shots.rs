// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Offscreen screenshots of the first-run states on the daemon source, in
//! light and dark, with the developer panel closed.
//!
//! Not part of `cargo test`; run explicitly:
//!
//! ```sh
//! RAMFLUX_SHOTS_DIR=/some/dir RAMFLUX_RF_BIN=/path/to/bin/rf \
//!   cargo test -p ramflux-desktop-ui --test first_run_shots
//! ```
//!
//! States: no daemon with the start offer, no daemon program, a start in
//! progress, a daemon that exited at once, and (with `RAMFLUX_RF_BIN`) a real
//! daemon started from the app's own button and attached, with Settings ›
//! Diagnostics showing it. Every daemon and directory made here is removed.

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    shots::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("first-run screenshots: skipped; GPUI has no headless renderer on this platform");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod shots {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use gpui_kit::assets::Assets;
    use gpui_kit::base::Root;
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Bounds, Entity, HeadlessAppContext, Point, ScrollDelta, WindowBounds,
        WindowHandle, WindowOptions, point, px, size,
    };
    use ramflux_desktop_core::SourceStatus;
    use ramflux_desktop_sdk::SdkConfig;
    use ramflux_desktop_sdk::local::LaunchStore;
    use ramflux_desktop_ui::Shell;
    use ramflux_desktop_ui::launch::{LaunchOptions, LaunchState};
    use ramflux_desktop_ui::settings::SettingsSection;
    use ramflux_desktop_ui::shell::{ShellOptions, SourceChoice};

    struct Shots {
        cx: HeadlessAppContext,
        out: PathBuf,
        dir: PathBuf,
    }

    struct Window {
        handle: WindowHandle<Root>,
        shell: Entity<Shell>,
    }

    impl Shots {
        fn window(&mut self, tag: &str, bundled: Option<PathBuf>) -> Window {
            let dir = self.dir.join(tag);
            std::fs::create_dir_all(&dir).expect("dir");
            let config = SdkConfig {
                reconnect_backoff: Duration::from_millis(200),
                poll_fast: Duration::from_millis(200),
                ..SdkConfig::new(dir.join("rfd.sock"))
            };
            let options = ShellOptions {
                source: SourceChoice::Rfd(config),
                launch: LaunchOptions {
                    store: LaunchStore::at(dir.join("desktop-settings.json")),
                    bundled,
                    log_dir: Some(dir.join("logs")),
                    owned: ramflux_desktop_sdk::local::OwnedDaemonFile::at(
                        dir.join("owned-daemon.json"),
                    ),
                },
                ..ShellOptions::for_scenario("", true)
            };
            let (handle, shell) = self
                .cx
                .update(|cx| {
                    let bounds =
                        Bounds { origin: Point::default(), size: size(px(1280.), px(820.)) };
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
            let w = Window { handle, shell };
            self.tick(5);
            w
        }

        /// Lets real time pass for child processes and the bridge while the
        /// test clock moves.
        fn tick(&mut self, rounds: usize) {
            for _ in 0..rounds {
                std::thread::sleep(Duration::from_millis(20));
                self.cx.advance_clock(Duration::from_millis(100));
                self.cx.run_until_parked();
            }
        }

        fn until(&mut self, what: &str, mut done: impl FnMut(&mut HeadlessAppContext) -> bool) {
            for _ in 0..600 {
                if done(&mut self.cx) {
                    return;
                }
                self.tick(1);
            }
            panic!("timed out waiting for: {what}");
        }

        fn in_window(
            &mut self,
            w: &Window,
            f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
        ) {
            self.cx
                .update_window(w.handle.into(), |_, window, cx| {
                    window.render_frame(cx);
                    f(window, cx);
                })
                .expect("window");
            self.cx.run_until_parked();
        }

        fn state(&mut self, w: &Window) -> LaunchState {
            self.cx.update(|cx| w.shell.read(cx).launcher().read(cx).state().clone())
        }

        fn both(&mut self, w: &Window, name: &str) {
            let dev = self.cx.update(|cx| w.shell.read(cx).is_dev_panel_visible());
            assert!(!dev, "screenshots are taken with the developer panel closed");
            for (mode, suffix) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
                self.cx.update(|cx| {
                    Theme::change(mode, None, cx);
                    cx.refresh_windows();
                });
                self.cx.run_until_parked();
                self.in_window(w, |_, _| {});
                let image = self.cx.capture_screenshot(w.handle.into()).expect("render");
                let path = self.out.join(format!("{name}_{suffix}.png"));
                image.save(&path).expect("write png");
                println!("wrote {}", path.display());
            }
        }

        fn stop(&mut self, w: &Window) {
            let launcher = self.cx.update(|cx| w.shell.read(cx).launcher().clone());
            self.cx.update(|cx| launcher.update(cx, |l, cx| l.stop_now(cx)));
        }
    }

    fn program(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("mode");
        path
    }

    pub fn run() {
        let out = std::env::var_os("RAMFLUX_SHOTS_DIR").map_or_else(
            || PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("shots"),
            PathBuf::from,
        );
        std::fs::create_dir_all(&out).expect("output directory");
        // Short: Unix socket paths are limited.
        let dir = PathBuf::from(format!("/tmp/rf-f5-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("work dir");
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            ramflux_desktop_ui::init(cx);
        });
        let mut s = Shots { cx, out, dir: dir.clone() };
        let waiting = program(&dir, "rf-wait", "exec sleep 60");
        let exiting = program(&dir, "rf-exit", "echo 'cannot bind socket' >&2\nexit 3");

        // 18. No daemon: where the app looks, and the start offer.
        let w = s.window("offer", Some(waiting.clone()));
        s.both(&w, "18_first_run_no_daemon_offer");

        // 19. No daemon program set: the offer is disabled, settings are named.
        let w = s.window("missing", None);
        s.both(&w, "19_first_run_no_program");

        // 20. A start in progress.
        let w = s.window("starting", Some(waiting));
        s.in_window(&w, |window, cx| window.click("entry-daemon-start", cx));
        s.tick(2);
        s.both(&w, "20_first_run_starting");
        s.stop(&w);

        // 21. A daemon that exits at once: the reason and a retry.
        let w = s.window("exit", Some(exiting));
        s.in_window(&w, |window, cx| window.click("entry-daemon-start", cx));
        s.until("exit seen", |cx| {
            cx.update(|cx| {
                matches!(w.shell.read(cx).launcher().read(cx).state(), LaunchState::Failed(_))
            })
        });
        s.both(&w, "21_first_run_daemon_exited");

        // 22, 23. A real daemon started from the app's button, attached.
        if let Some(rf) = std::env::var_os("RAMFLUX_RF_BIN").map(PathBuf::from) {
            let w = s.window("real", Some(rf));
            s.in_window(&w, |window, cx| window.click("entry-daemon-start", cx));
            s.until("attached", |cx| {
                cx.update(|cx| {
                    let store = w.shell.read(cx).store().read(cx).store();
                    store.source_status() == SourceStatus::Connected && store.daemon().is_some()
                })
            });
            s.until("running", |cx| {
                cx.update(|cx| {
                    matches!(
                        w.shell.read(cx).launcher().read(cx).state(),
                        LaunchState::Running { .. }
                    )
                })
            });
            s.tick(10);
            println!("page: {:?}", s.cx.update(|cx| w.shell.read(cx).page()));
            s.both(&w, "22_first_run_started_and_attached");
            let shell = w.shell.clone();
            s.cx.update(|cx| {
                shell.update(cx, |shell, cx| {
                    shell.open_settings_at(SettingsSection::Diagnostics, cx);
                })
            });
            s.tick(2);
            s.in_window(&w, |window, cx| {
                let delta = ScrollDelta::Pixels(point(px(0.), px(-900.)));
                window.scroll("renderer-profile", delta, cx);
            });
            s.both(&w, "23_settings_diagnostics_local_setup");
            s.stop(&w);
            assert_eq!(s.state(&w), LaunchState::Idle, "the started daemon is stopped");
        } else {
            println!("set RAMFLUX_RF_BIN for the shots of a real daemon (22, 23)");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
