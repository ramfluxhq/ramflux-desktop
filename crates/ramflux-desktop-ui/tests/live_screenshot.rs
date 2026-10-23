// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Offscreen screenshot of the production window root on a live daemon.
//!
//! Not part of `cargo test`; run explicitly against a running daemon:
//!
//! ```sh
//! RAMFLUX_SHOTS_SOCKET=/path/rfd.sock RAMFLUX_SHOTS_DIR=/some/dir \
//!   cargo test -p ramflux-desktop-ui --test live_screenshot
//! ```
//!
//! The developer panel stays closed. The harness waits until the entry page
//! shows the daemon's status, then saves light and dark renders.

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    shot::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("live screenshot: skipped; GPUI has no headless renderer on this platform");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod shot {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use gpui_kit::assets::Assets;
    use gpui_kit::base::Root;
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Bounds, HeadlessAppContext, Point, WindowBounds, WindowOptions, px, size,
    };
    use ramflux_desktop_core::SourceStatus;
    use ramflux_desktop_sdk::SdkConfig;
    use ramflux_desktop_ui::Shell;
    use ramflux_desktop_ui::shell::{ShellOptions, SourceChoice};
    use ramflux_desktop_ui::workspace::PrefsStore;

    pub fn run() {
        let Some(socket) = std::env::var_os("RAMFLUX_SHOTS_SOCKET") else {
            println!("live screenshot: set RAMFLUX_SHOTS_SOCKET to a running daemon's socket");
            return;
        };
        let out = std::env::var_os("RAMFLUX_SHOTS_DIR").map_or_else(
            || PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("shots"),
            PathBuf::from,
        );
        std::fs::create_dir_all(&out).expect("output directory");
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            ramflux_desktop_ui::init(cx);
        });
        let options = ShellOptions {
            source: SourceChoice::Rfd(SdkConfig::new(PathBuf::from(socket))),
            scenario: String::new(),
            start_clock: false,
            wall_clock: true,
            prefs: PrefsStore::in_memory(),
        };
        let (handle, shell) = cx
            .update(|cx| {
                let bounds = Bounds { origin: Point::default(), size: size(px(1440.), px(960.)) };
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
        let store = cx.update(|cx| shell.read(cx).store().clone());
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            cx.update(|cx| store.update(cx, |store, cx| store.advance(50, cx)));
            cx.run_until_parked();
            let ready = cx.update(|cx| {
                let s = store.read(cx).store();
                s.source_status() == SourceStatus::Connected && s.daemon().is_some()
            });
            if ready {
                break;
            }
            assert!(Instant::now() < deadline, "the daemon did not answer");
            std::thread::sleep(Duration::from_millis(50));
        }
        let page = cx.update(|cx| shell.read(cx).page());
        let dev = cx.update(|cx| shell.read(cx).is_dev_panel_visible());
        assert!(!dev, "screenshots are taken with the developer panel closed");
        println!("page: {page:?}");
        for (mode, name) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
            cx.update(|cx| {
                Theme::change(mode, None, cx);
                cx.refresh_windows();
            });
            cx.run_until_parked();
            cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx)).expect("frame");
            let image = cx.capture_screenshot(handle.into()).expect("offscreen render");
            let path = out.join(format!("01_live_entry_node_step_{name}.png"));
            image.save(&path).expect("write png");
            println!("wrote {}", path.display());
        }
    }
}
