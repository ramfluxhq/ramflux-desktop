// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Offscreen light and dark screenshots of contact requests in the
//! contacts dock on a fake daemon with the CONTACT-02 request API
//! (contract-first): a pending request with its note, held messages and
//! safety number, the auto-accept setting, and a contact that has not
//! accepted this account's request yet.
//!
//! Not part of `cargo test`; run explicitly (`RAMFLUX_SHOTS_LOCALE` picks
//! the interface language, `en` by default):
//!
//! ```sh
//! RAMFLUX_SHOTS_DIR=/some/dir cargo test -p ramflux-desktop-ui --test request_shots
//! ```

mod common;
mod sdk_fake;

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    shots::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("request shots: skipped; GPUI has no headless renderer on this platform");
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
    use ramflux_desktop_core::{ContextId, ParticipantId, Store, selectors};
    use ramflux_desktop_ui::AppAssets as Assets;
    use ramflux_desktop_ui::Shell;
    use ramflux_desktop_ui::route::Page;

    use crate::sdk_fake;

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
            || PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("request-shots"),
            PathBuf::from,
        );
        let locale = std::env::var("RAMFLUX_SHOTS_LOCALE").unwrap_or_else(|_| "en".to_owned());
        std::fs::create_dir_all(&out).expect("output directory");
        let (fake, state, socket) = sdk_fake::start(&format!("f21-shots-{locale}"));
        fake.enable_stream();
        let fake = Arc::new(fake);
        let requests = state.lock().expect("state").requests.clone();
        let bus = Arc::downgrade(&fake);
        requests.set_publisher(Box::new(move |account, topic, body| {
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
        let options = sdk_fake::options(&socket);
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
        let mut run = Run { cx, handle, shell, out, locale };
        run.until("the workspace", |r| {
            r.cx.update(|cx| r.shell.read(cx).page()) == Page::Workspace
        });
        run.until("the account", |r| r.store(|s| s.context(&ContextId::new("dm-ana")).is_some()));
        requests.outgoing("acct-main", "principal-ana");
        fake.publish("acct-main", "contact.changed", serde_json::json!({}));
        let id = requests.incoming(
            "acct-main",
            "principal-mira",
            Some("We met at the RustConf hallway track"),
        );
        requests.hold(&id, "Hi! Here are the slides I mentioned.");
        requests.hold(&id, "And the repo link.");
        run.until("the request", |r| {
            r.store(|s| {
                selectors::pending_contact_requests(s).first().is_some_and(|q| q.held_messages == 2)
                    && selectors::outgoing_contact_request(s, &ParticipantId::new("principal-ana"))
                        .is_some()
            })
        });
        run.click("rail-contacts");
        run.settle();
        run.shot("f21_01_request");
        run.click("contacts-auto-accept");
        run.until("auto accept", |r| {
            r.store(|s| s.request_policy() == Some(ramflux_desktop_core::RequestPolicy::AutoAccept))
        });
        run.settle();
        run.shot("f21_02_auto_accept");
    }
}
