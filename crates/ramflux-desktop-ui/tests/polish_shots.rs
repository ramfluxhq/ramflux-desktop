// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Offscreen screenshots of the conversation view for a design review, at
//! 1280×800 and 1920×1080, in Simplified Chinese and English, light and
//! dark, with the developer panel closed: a day of messages (scenario
//! s02), and an AI participant's task that needs the user and then is done
//! (scenario s10).
//!
//! The window is built in the default locale (Simplified Chinese) and every
//! language is switched to while it is open, as Settings › Appearance does,
//! so the shots also show what follows a switch.
//!
//! Not part of `cargo test`; run explicitly:
//!
//! ```sh
//! RAMFLUX_SHOTS_DIR=/some/dir cargo test -p ramflux-desktop-ui --test polish_shots
//! ```

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    shots::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("polish shots: skipped; GPUI has no headless renderer on this platform");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod shots {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use gpui_kit::base::Root;
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
    use gpui_kit::{
        AppContext as _, Bounds, Entity, HeadlessAppContext, MouseButton, Point, Window,
        WindowBounds, WindowHandle, WindowOptions, px, size,
    };
    use ramflux_desktop_core::{ContextId, TaskId, TaskState};
    use ramflux_desktop_ui::AppAssets as Assets;
    use ramflux_desktop_ui::Shell;
    use ramflux_desktop_ui::confirm::HOLD_DURATION;
    use ramflux_desktop_ui::i18n;

    /// The window sizes reviewed, and their file name tags.
    const SIZES: [(f32, f32, &str); 2] = [(1280., 800., "1280"), (1920., 1080., "1920")];

    struct Run {
        cx: HeadlessAppContext,
        handle: WindowHandle<Root>,
        shell: Entity<Shell>,
        out: PathBuf,
        tag: &'static str,
    }

    impl Run {
        fn open(scenario: &str, width: f32, height: f32, tag: &'static str, out: &Path) -> Self {
            let mut cx = HeadlessAppContext::with_platform(
                gpui_kit::platform::current_platform(true).text_system(),
                Arc::new(Assets),
                gpui_kit::platform::current_headless_renderer,
            );
            cx.update(|cx| {
                gpui_kit::init(cx);
                ramflux_desktop_ui::init(cx);
                Theme::change(ThemeMode::Light, None, cx);
            });
            let name = scenario.to_owned();
            let (handle, shell) = cx
                .update(|cx| {
                    let bounds =
                        Bounds { origin: Point::default(), size: size(px(width), px(height)) };
                    gpui_kit::open_window(
                        WindowOptions {
                            window_bounds: Some(WindowBounds::Windowed(bounds)),
                            focus: false,
                            show: false,
                            ..Default::default()
                        },
                        cx,
                        move |window, cx| {
                            cx.new(|cx| Shell::with_scenario(&name, true, window, cx))
                        },
                    )
                })
                .expect("offscreen window");
            let handle = handle.downcast::<Root>().expect("Root");
            Self { cx, handle, shell, out: out.to_path_buf(), tag }
        }

        /// Moves the scenario clock forward by `ms`.
        fn advance(&mut self, ms: u64) {
            self.cx.advance_clock(Duration::from_millis(ms));
            self.cx.run_until_parked();
        }

        fn window<R>(&mut self, f: impl FnOnce(&mut Window, &mut gpui_kit::App) -> R) -> R {
            let result = self
                .cx
                .update_window(self.handle.into(), |_, window, cx| {
                    window.render_frame(cx);
                    f(window, cx)
                })
                .expect("window");
            self.cx.run_until_parked();
            result
        }

        fn show(&mut self, context: &str) {
            let workspace = self.cx.update(|cx| self.shell.read(cx).workspace().clone());
            let id = ContextId::new(context);
            self.window(move |window, cx| {
                workspace.update(cx, |w, cx| w.select_context(Some(id), window, cx));
            });
        }

        fn click_in(&mut self, scope: &str, id: &str) {
            let (scope, id) = (scope.to_owned(), id.to_owned());
            self.window(move |window, cx| window.within(scope).click(id, cx));
        }

        fn task_state(&mut self, id: &str) -> Option<TaskState> {
            let id = TaskId::new(id);
            self.cx.update(|cx| {
                self.shell.read(cx).store().read(cx).store().task(&id).map(|t| t.state)
            })
        }

        /// Saves `name` in Chinese and English, light and dark; the locale
        /// is switched while the window is open.
        fn shot(&mut self, name: &str) {
            let dev = self.cx.update(|cx| self.shell.read(cx).is_dev_panel_visible());
            assert!(!dev, "screenshots are taken with the developer panel closed");
            for (locale, lang) in [("zh-CN", "zh"), ("en", "en")] {
                for (mode, suffix) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
                    self.cx.update(|cx| {
                        i18n::select_locale(locale, cx);
                        Theme::change(mode, None, cx);
                        cx.refresh_windows();
                    });
                    self.window(|_, _| {});
                    let image = self.cx.capture_screenshot(self.handle.into()).expect("render");
                    let path = self.out.join(format!("{name}_{}_{lang}_{suffix}.png", self.tag));
                    image.save(&path).expect("write png");
                    println!("wrote {}", path.display());
                }
            }
            self.cx.update(|cx| {
                i18n::select_locale(i18n::DEFAULT_LOCALE, cx);
                Theme::change(ThemeMode::Light, None, cx);
                cx.refresh_windows();
            });
            self.cx.run_until_parked();
        }
    }

    /// A day of messages in a group.
    fn daily(width: f32, height: f32, tag: &'static str, out: &Path) {
        let mut run = Run::open("s02_daily", width, height, tag, out);
        run.advance(3_000);
        run.show("c-team");
        run.advance(500);
        run.shot("f15_01_conversation");
    }

    /// Orbit's task: asked, started, needs the user, approved with the
    /// hold, done.
    fn task(width: f32, height: f32, tag: &'static str, out: &Path) {
        let mut run = Run::open("s10_ai_participant", width, height, tag, out);
        run.advance(200);
        run.show("c-launch");
        run.window(|window, cx| {
            window.click("composer-input", cx);
            window.input("@Orbit can you check the launch checklist?", cx);
        });
        run.window(|window, cx| window.click("composer-send", cx));
        run.advance(1_100);
        assert_eq!(run.task_state("t-launch"), Some(TaskState::Proposed));
        run.click_in("moment-m-task", "task-start");
        run.advance(1_400);
        assert_eq!(run.task_state("t-launch"), Some(TaskState::NeedsMe));
        run.shot("f15_02_task_needs_me");
        run.click_in("moment-m-task", "step-approve");
        let center = run.window(|window, cx| {
            let center = window.find("hold-confirm").bounds().center();
            window.pointer_down(center, MouseButton::Left, cx);
            center
        });
        run.advance(u64::try_from(HOLD_DURATION.as_millis()).unwrap_or(0) + 200);
        run.window(|window, cx| window.pointer_up(center, MouseButton::Left, cx));
        run.advance(1_600);
        assert_eq!(run.task_state("t-launch"), Some(TaskState::Done));
        run.shot("f15_03_task_done");
    }

    pub fn run() {
        let out = std::env::var_os("RAMFLUX_SHOTS_DIR").map_or_else(
            || PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("polish-shots"),
            PathBuf::from,
        );
        std::fs::create_dir_all(&out).expect("output directory");
        for (width, height, tag) in SIZES {
            daily(width, height, tag, &out);
            task(width, height, tag, &out);
        }
    }
}
