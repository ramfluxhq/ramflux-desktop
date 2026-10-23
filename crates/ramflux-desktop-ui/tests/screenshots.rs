// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Renders reference screenshots of the production window root with GPUI's
//! offscreen renderer (Metal on macOS, WGPU on Linux).
//!
//! Not part of `cargo test`; run explicitly:
//!
//! ```sh
//! RAMFLUX_SHOTS_DIR=/some/dir cargo test -p ramflux-desktop-ui --test screenshots
//! ```
//!
//! Scenarios are switched through the developer panel buttons, exactly as a
//! person would, and time is moved with the panel's seek control.

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    shots::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("screenshots: skipped; GPUI has no headless renderer on this platform");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod shots {
    use std::path::PathBuf;
    use std::sync::Arc;

    use gpui_kit::assets::Assets;
    use gpui_kit::base::Root;
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Bounds, Entity, HeadlessAppContext, Point, WindowBounds, WindowHandle,
        WindowOptions, px, size,
    };
    use ramflux_desktop_ui::Shell;
    use ramflux_desktop_ui::actions::ToggleDevPanel;

    struct Session {
        cx: HeadlessAppContext,
        handle: WindowHandle<Root>,
        shell: Entity<Shell>,
        out: PathBuf,
    }

    impl Session {
        fn open(out: PathBuf) -> Self {
            let mut cx = HeadlessAppContext::with_platform(
                gpui_kit::platform::current_platform(true).text_system(),
                Arc::new(Assets),
                gpui_kit::platform::current_headless_renderer,
            );
            cx.update(|cx| {
                gpui_kit::init(cx);
                ramflux_desktop_ui::init(cx);
            });
            let (handle, shell) = cx
                .update(|cx| {
                    let bounds =
                        Bounds { origin: Point::default(), size: size(px(1440.), px(860.)) };
                    gpui_kit::open_window(
                        WindowOptions {
                            window_bounds: Some(WindowBounds::Windowed(bounds)),
                            focus: false,
                            show: false,
                            ..Default::default()
                        },
                        cx,
                        |window, cx| {
                            cx.new(|cx| Shell::with_scenario("s01_first_launch", false, window, cx))
                        },
                    )
                })
                .expect("offscreen window");
            let handle = handle.downcast::<Root>().expect("Root");
            Self { cx, handle, shell, out }
        }

        fn in_window(&mut self, f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App)) {
            self.cx
                .update_window(self.handle.into(), |_, window, cx| {
                    window.render_frame(cx);
                    f(window, cx);
                })
                .expect("window");
            self.cx.run_until_parked();
        }

        fn click(&mut self, id: &'static str) {
            self.in_window(|window, cx| window.click(id, cx));
        }

        fn seek(&mut self, ms: u64) {
            let dev = self.cx.update(|cx| self.shell.read(cx).dev_panel().cloned());
            let dev = dev.expect("developer panel (debug build)");
            self.cx.update(|cx| dev.update(cx, |dev, cx| dev.seek(ms, cx)));
            self.cx.run_until_parked();
        }

        fn theme(&mut self, mode: ThemeMode) {
            self.cx.update(|cx| {
                Theme::change(mode, None, cx);
                cx.refresh_windows();
            });
            self.cx.run_until_parked();
        }

        fn save(&mut self, name: &str) {
            self.in_window(|_, _| {});
            let image = self.cx.capture_screenshot(self.handle.into()).expect("offscreen render");
            let path = self.out.join(name);
            image.save(&path).expect("write png");
            println!("wrote {}", path.display());
        }
    }

    pub fn run() {
        let out = std::env::var_os("RAMFLUX_SHOTS_DIR").map_or_else(
            || PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("shots"),
            PathBuf::from,
        );
        std::fs::create_dir_all(&out).expect("output directory");
        let mut s = Session::open(out);
        s.in_window(|window, cx| window.dispatch_action(Box::new(ToggleDevPanel), cx));

        // s01: entry at the account step, light.
        s.theme(ThemeMode::Light);
        s.click("dev-scenario-s01_first_launch");
        s.seek(4_500);
        s.save("01_s01_entry_light.png");

        // s02: daily workspace, light and dark.
        s.click("dev-scenario-s02_daily");
        s.seek(3_000);
        s.save("02_s02_workspace_light.png");
        s.theme(ThemeMode::Dark);
        s.save("03_s02_workspace_dark.png");

        // s03: approval with the hold-to-confirm strip open, dark and light.
        s.click("dev-scenario-s03_intervention");
        s.seek(1_000);
        s.in_window(|window, cx| window.within("moment-m-approve").click("details", cx));
        s.in_window(|window, cx| window.within("moment-m-approve").click("approve", cx));
        s.save("04_s03_confirm_dark.png");
        s.theme(ThemeMode::Light);
        s.save("05_s03_confirm_light.png");
    }
}
