// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Offscreen light and dark screenshots of the states DESK-PIN-03 adds or
//! changes, on a fake daemon: the title bar with the longest failure at the
//! application's minimum window size and at 1100 px, the failure chip
//! without a redundant prefix, and the contact-request and group refusals.
//!
//! Not part of `cargo test`; run explicitly (`RAMFLUX_SHOTS_LOCALE` picks
//! the interface language, `en` by default):
//!
//! ```sh
//! RAMFLUX_SHOTS_DIR=/some/dir RAMFLUX_SHOTS_LOCALE=zh-CN \
//!   cargo test -p ramflux-desktop-ui --test desk_pin3_shots
//! ```

mod common;
mod sdk_fake;

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    shots::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("desk pin 3 shots: skipped; GPUI has no headless renderer on this platform");
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
        AppContext as _, Bounds, Entity, HeadlessAppContext, Point, Window, WindowBounds,
        WindowHandle, WindowOptions, px, size,
    };
    use ramflux_desktop_core::{ContextId, Intent, IntentKind, RequestAnswer};
    use ramflux_desktop_ui::AppAssets as Assets;
    use ramflux_desktop_ui::Shell;
    use ramflux_desktop_ui::route::Page;
    use ramflux_desktop_ui::shell::ShellOptions;

    use crate::sdk_fake;

    struct Run {
        width: f32,
        height: f32,
        cx: HeadlessAppContext,
        handle: WindowHandle<Root>,
        shell: Entity<Shell>,
        out: PathBuf,
        locale: String,
    }

    impl Run {
        fn open(
            options: ShellOptions,
            out: PathBuf,
            locale: String,
            width: f32,
            height: f32,
        ) -> Self {
            let mut cx = HeadlessAppContext::with_platform(
                gpui_kit::platform::current_platform(true).text_system(),
                Arc::new(Assets),
                gpui_kit::platform::current_headless_renderer,
            );
            cx.update(|cx| {
                gpui_kit::init(cx);
                ramflux_desktop_ui::init(cx);
                ramflux_desktop_ui::i18n::select_locale(&locale, cx);
            });
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
                        |window, cx| cx.new(|cx| Shell::with_options(options, window, cx)),
                    )
                })
                .expect("offscreen window");
            let handle = handle.downcast::<Root>().expect("Root");
            Self { width, height, cx, handle, shell, out, locale }
        }

        fn tick(&mut self) {
            std::thread::sleep(Duration::from_millis(10));
            self.cx.advance_clock(Duration::from_millis(50));
            self.cx.run_until_parked();
        }

        fn settle(&mut self) {
            for _ in 0..10 {
                self.tick();
            }
        }

        fn until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
            let limit = Instant::now() + Duration::from_secs(30);
            while !done(self) {
                assert!(Instant::now() < limit, "timed out waiting for {what}");
                self.tick();
            }
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

        fn click(&mut self, id: &str) {
            let id = id.to_owned();
            self.window(move |window, cx| window.click(id, cx));
        }

        fn click_in(&mut self, scope: &str, id: &str) {
            let (scope, id) = (scope.to_owned(), id.to_owned());
            self.window(move |window, cx| window.within(scope).click(id, cx));
        }

        fn type_into(&mut self, id: &str, text: &str) {
            let (id, text) = (id.to_owned(), text.to_owned());
            self.window(move |window, cx| {
                window.click(id.clone(), cx);
                window.press("secondary-a", cx);
                window.press("backspace", cx);
                window.input(&text, cx);
            });
        }

        fn exists(&mut self, id: &str) -> bool {
            let id = id.to_owned();
            self.window(move |window, _| !window.find_all(id).is_empty())
        }

        fn store<R>(&mut self, f: impl FnOnce(&ramflux_desktop_core::Store) -> R) -> R {
            self.cx.update(|cx| f(self.shell.read(cx).store().read(cx).store()))
        }

        fn show(&mut self, id: &str) {
            let workspace = self.cx.update(|cx| self.shell.read(cx).workspace().clone());
            let id = ContextId::new(id);
            self.cx
                .update_window(self.handle.into(), |_, window, cx| {
                    workspace.update(cx, |w, cx| w.select_context(Some(id), window, cx));
                })
                .expect("window");
            self.cx.run_until_parked();
        }

        /// Accepts request `id` as the dock's Accept button does. A window
        /// of the minimum size is too narrow to click the dock's button.
        fn accept(&mut self, id: &str) {
            let intent = Intent::human(IntentKind::AnswerContactRequest {
                request_id: id.to_owned(),
                answer: RequestAnswer::Accept,
            });
            let store = self.cx.update(|cx| self.shell.read(cx).store().clone());
            self.cx.update(|cx| {
                store.update(cx, |handle, cx| {
                    handle.dispatch(intent, cx);
                });
            });
            self.cx.run_until_parked();
        }

        fn shot(&mut self, name: &str) {
            let name = format!("{name}_{}x{}", self.width, self.height);
            for (mode, suffix) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
                self.cx.update(|cx| {
                    Theme::change(mode, None, cx);
                    cx.refresh_windows();
                });
                self.window(|_, _| {});
                let image = self.cx.capture_screenshot(self.handle.into()).expect("render");
                let path = self.out.join(format!("{name}_{}_{suffix}.png", self.locale));
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

    fn out_dir() -> (PathBuf, String) {
        let out = std::env::var_os("RAMFLUX_SHOTS_DIR").map_or_else(
            || PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("desk-pin3-shots"),
            PathBuf::from,
        );
        let locale = std::env::var("RAMFLUX_SHOTS_LOCALE").unwrap_or_else(|_| "en".to_owned());
        std::fs::create_dir_all(&out).expect("output directory");
        (out, locale)
    }

    pub fn run() {
        let (out, locale) = out_dir();
        title_bar_states(&out, &locale);
    }

    /// The longest failure the strip can show, at the minimum window size and
    /// at 1100 px: the trailing controls stay.
    fn title_bar_states(out: &std::path::Path, locale: &str) {
        let (min_width, min_height) = ramflux_desktop_ui::MIN_WINDOW_SIZE;
        for (width, height) in [(1100., 760.), (min_width, min_height)] {
            let (fake, state, socket, _tasks) =
                sdk_fake::start_with_tasks(&format!("desk-pin3-title-{locale}-{width}"));
            let requests = state.lock().expect("state").requests.clone();
            let bus = Arc::new(fake);
            let weak = Arc::downgrade(&bus);
            requests.set_publisher(Box::new(move |a, t, b| {
                if let Some(bus) = weak.upgrade() {
                    bus.publish(a, t, b);
                }
            }));
            let id = requests.incoming("acct-main", "principal-mira", None);
            let options = sdk_fake::options(&socket);
            let mut run = Run::open(options, out.to_path_buf(), locale.to_owned(), width, height);
            run.until("the workspace", |r| {
                r.cx.update(|cx| r.shell.read(cx).page()) == Page::Workspace
            });
            bus.publish("acct-main", "contact.changed", serde_json::json!({}));
            run.until("the request", |r| {
                r.store(|s| {
                    !ramflux_desktop_core::selectors::pending_contact_requests(s).is_empty()
                })
            });
            // The longest refusal text the strip can show: a failed trust check.
            requests.fail_decisions(Some(("TransportSecurity", "node certificate pin mismatch")));
            run.accept(&id);
            run.until("the refusal", |r| r.store(|s| !s.failures().is_empty()));
            run.settle();
            run.shot("pin3_01_title_bar_long_failure");
            if let Ok(bus) = Arc::try_unwrap(bus) {
                bus.shutdown();
            }
        }
    }
}
