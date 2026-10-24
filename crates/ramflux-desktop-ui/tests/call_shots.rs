// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Offscreen light and dark screenshots of voice calls on a fake daemon
//! with the CALL-01 call API and stand-in audio: the call button, calling,
//! an active call with its quality, a blocked microphone, the incoming-call
//! card, and the call records in the timeline.
//!
//! Not part of `cargo test`; run explicitly (`RAMFLUX_SHOTS_LOCALE` picks
//! the interface language, `en` by default):
//!
//! ```sh
//! RAMFLUX_SHOTS_DIR=/some/dir cargo test -p ramflux-desktop-ui --test call_shots
//! ```

mod common;
mod sdk_fake;

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    shots::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("call shots: skipped; GPUI has no headless renderer on this platform");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod shots {
    use std::cell::Cell;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use gpui_kit::base::Root;
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Bounds, Entity, HeadlessAppContext, Point, Window, WindowBounds,
        WindowHandle, WindowOptions, px, size,
    };
    use ramflux_desktop_core::{CallId, CallMedia, CallState, ContextId, Store, selectors};
    use ramflux_desktop_ui::AppAssets as Assets;
    use ramflux_desktop_ui::Shell;
    use ramflux_desktop_ui::app::{CallAudio, CallQuality, LiveCall, LiveStatus};
    use ramflux_desktop_ui::route::Page;
    use ramflux_desktop_ui::shell::SourceChoice;

    use crate::sdk_fake;

    /// Stand-in audio whose report a scene sets.
    #[derive(Clone, Default)]
    struct Audio(Rc<Cell<Option<LiveStatus>>>);

    struct Live(Rc<Cell<Option<LiveStatus>>>);

    impl LiveCall for Live {
        fn set_muted(&mut self, _: bool) {}

        fn status(&self) -> LiveStatus {
            self.0
                .get()
                .unwrap_or(LiveStatus { quality: CallQuality::Good, microphone_blocked: false })
        }
    }

    impl CallAudio for Audio {
        fn start(&self, _: CallMedia) -> Result<Box<dyn LiveCall>, String> {
            Ok(Box::new(Live(self.0.clone())))
        }

        fn ring(&self) -> Option<Box<dyn std::any::Any>> {
            None
        }
    }

    struct Run {
        cx: HeadlessAppContext,
        handle: WindowHandle<Root>,
        shell: Entity<Shell>,
        out: PathBuf,
        locale: String,
    }

    impl Run {
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

        fn store<R>(&mut self, f: impl FnOnce(&Store) -> R) -> R {
            self.cx.update(|cx| f(self.shell.read(cx).store().read(cx).store()))
        }

        fn state(&mut self, id: &CallId) -> Option<CallState> {
            self.store(|s| s.call(id).map(|c| c.state))
        }

        fn shot(&mut self, name: &str) {
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

    pub fn run() {
        let out = std::env::var_os("RAMFLUX_SHOTS_DIR").map_or_else(
            || PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("call-shots"),
            PathBuf::from,
        );
        let locale = std::env::var("RAMFLUX_SHOTS_LOCALE").unwrap_or_else(|_| "en".to_owned());
        std::fs::create_dir_all(&out).expect("output directory");
        let (fake, state, socket) = sdk_fake::start(&format!("f16-shots-{locale}"));
        fake.enable_stream();
        let fake = Arc::new(fake);
        let calls = state.lock().expect("state").calls.clone();
        let bus = Arc::downgrade(&fake);
        calls.set_publisher(Box::new(move |account, topic, body| {
            if let Some(bus) = bus.upgrade() {
                bus.publish(account, topic, body);
            }
        }));
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
        let mut options = sdk_fake::options(&socket);
        if let SourceChoice::Rfd(config) = &mut options.source {
            config.call_api = true;
        }
        let (handle, shell) = cx
            .update(|cx| {
                let bounds = Bounds { origin: Point::default(), size: size(px(1280.), px(860.)) };
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
        let audio = Audio::default();
        let installed = audio.clone();
        cx.update(|cx| {
            let store = shell.read(cx).store().clone();
            store.update(cx, |store, _| store.set_call_audio(Rc::new(installed)));
        });
        let mut run = Run { cx, handle, shell, out, locale };
        run.until("the workspace", |r| {
            r.cx.update(|cx| r.shell.read(cx).page()) == Page::Workspace
        });
        run.until("the account", |r| r.store(|s| s.context(&ContextId::new("dm-ana")).is_some()));
        let workspace = run.cx.update(|cx| run.shell.read(cx).workspace().clone());
        run.window(|window, cx| {
            workspace.update(cx, |w, cx| {
                w.select_context(Some(ContextId::new("dm-ana")), window, cx);
            });
        });
        run.settle();
        run.shot("f16_01_call_button");

        // A missed and a declined call first, so the timeline has records.
        let missed = CallId::new(calls.incoming("acct-main", "dm-ana"));
        run.until("ringing", |r| r.state(&missed) == Some(CallState::IncomingRinging));
        run.settle();
        run.shot("f16_02_incoming_card");
        calls.advance(Duration::from_secs(45));
        run.until("missed", |r| r.state(&missed).is_some_and(CallState::is_ended));

        run.click("conversation-call");
        run.until("calling", |r| r.store(|s| selectors::current_call(s).is_some()));
        run.settle();
        run.shot("f16_03_calling");
        let id = run.store(|s| selectors::current_call(s).map(|c| c.id.clone())).expect("call");
        calls.peer_answers(id.as_str());
        run.until("active", |r| r.state(&id) == Some(CallState::Active));
        run.settle();
        run.shot("f16_04_active");
        audio.0.set(Some(LiveStatus { quality: CallQuality::Fair, microphone_blocked: true }));
        for _ in 0..25 {
            run.tick();
        }
        run.shot("f16_05_microphone_blocked");
        audio.0.set(None);
        run.click("call-hang-up");
        run.until("ended", |r| r.state(&id).is_some_and(CallState::is_ended));
        calls.advance(Duration::from_secs(60));
        let declined = CallId::new(calls.incoming("acct-main", "dm-ana"));
        run.until("ringing", |r| r.state(&declined) == Some(CallState::IncomingRinging));
        run.settle();
        run.click("incoming-decline");
        run.until("declined", |r| r.state(&declined).is_some_and(CallState::is_ended));
        run.settle();
        run.shot("f16_06_records");

        if let Ok(fake) = Arc::try_unwrap(fake) {
            fake.shutdown();
        }
    }
}
