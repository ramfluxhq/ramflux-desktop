// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Offscreen light and dark screenshots of adding contacts (by contact
//! card, and by principal id: unverified), sharing this account's card, a
//! new group, and the first direct message to a freshly added contact, on a
//! fake daemon that resolves contacts when they are added (see
//! `resolve_fake`), with the developer panel closed.
//!
//! Not part of `cargo test`; run explicitly (`RAMFLUX_SHOTS_LOCALE` picks
//! the interface language, `en` by default):
//!
//! ```sh
//! RAMFLUX_SHOTS_DIR=/some/dir RAMFLUX_SHOTS_LOCALE=zh-CN \
//!   cargo test -p ramflux-desktop-ui --test contacts_shots
//! ```

mod common;
mod resolve_fake;
mod sdk_fake;

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    shots::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("contacts shots: skipped; GPUI has no headless renderer on this platform");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod shots {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use gpui_kit::assets::Assets;
    use gpui_kit::base::Root;
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
    use gpui_kit::{
        AppContext as _, Bounds, Entity, HeadlessAppContext, MouseButton, Point, Window,
        WindowBounds, WindowHandle, WindowOptions, px, size,
    };
    use ramflux_desktop_core::{ContextId, ParticipantId};
    use ramflux_desktop_ui::Shell;
    use ramflux_desktop_ui::i18n::tr;
    use ramflux_desktop_ui::route::Page;

    use crate::{resolve_fake, sdk_fake};

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

        fn click_label(&mut self, label: &str) {
            let label = label.to_owned();
            self.window(move |window, cx| {
                let at = window.find_by_label(&label).bounds().center();
                window.pointer_down(at, MouseButton::Left, cx);
                window.pointer_up(at, MouseButton::Left, cx);
            });
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

        fn store<R>(&mut self, f: impl FnOnce(&ramflux_desktop_core::Store) -> R) -> R {
            self.cx.update(|cx| f(self.shell.read(cx).store().read(cx).store()))
        }

        fn current(&mut self) -> Option<ContextId> {
            self.cx.update(|cx| self.shell.read(cx).workspace().read(cx).current_context().cloned())
        }

        fn reachable(&mut self, who: &str) -> bool {
            let who = ParticipantId::new(who);
            self.store(|s| s.participant(&who).is_some_and(|p| p.reachable.is_some()))
        }

        fn shot(&mut self, name: &str) {
            let dev = self.cx.update(|cx| self.shell.read(cx).is_dev_panel_visible());
            assert!(!dev, "screenshots are taken with the developer panel closed");
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
            || PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("contacts-shots"),
            PathBuf::from,
        );
        let locale = std::env::var("RAMFLUX_SHOTS_LOCALE").unwrap_or_else(|_| "en".to_owned());
        std::fs::create_dir_all(&out).expect("output directory");
        let (fake, _state, socket) = resolve_fake::start(&format!("f14-shots-{locale}"));
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
        run.click("rail-contacts");

        // Adding by principal id: trusted on first use, unverified.
        run.type_into("contact-add-id", "principal-kai");
        run.shot("f14_01_add_by_id_typed");
        run.click("contact-add");
        run.until("kai", |r| r.reachable("principal-kai"));
        run.settle();
        run.shot("f14_02_added_by_id_unverified");

        // Adding by contact card.
        run.type_into("contact-add-id", "ramflux:contact/principal-bot#sig-bot");
        run.shot("f14_03_add_by_card_typed");
        run.click("contact-add");
        run.until("the card's contact", |r| r.reachable("principal-bot"));
        run.settle();
        run.shot("f14_04_added_by_card");
        run.type_into("contact-add-id", "ramflux:contact/forged#sig");
        run.click("contact-add");
        run.until("the refusal", |r| r.store(|s| !s.failures().is_empty()));
        run.settle();
        run.shot("f14_05_card_refused");
        run.type_into("contact-add-id", "principal-lu");
        run.click("contact-add");
        run.until("lu", |r| r.reachable("principal-lu"));
        run.settle();

        // This account's card.
        run.click("contact-card-show");
        run.until("the card", |r| {
            r.window(|window, _| window.try_find("contact-card-text").is_some())
        });
        run.settle();
        run.shot("f14_06_my_card");
        run.click("contact-card-hide");

        // First direct message to the contact added by id.
        run.window(|window, cx| window.within("contacts-principal-kai").click("message", cx));
        run.click("rail-contacts");
        run.type_into("composer-input", "Hello Kai, I added you by your id");
        run.shot("f14_07_first_dm_draft");
        run.click("composer-send");
        run.until("the conversation", |r| r.current().is_some());
        run.settle();
        run.shot("f14_08_first_dm_started");

        // A new group.
        run.click("stack-new");
        run.click_label(&tr("stack.new_group"));
        run.type_into("new-group-name", "Weekend trip");
        run.click("new-group-member-principal-kai");
        run.click("new-group-member-principal-bot");
        run.shot("f14_09_new_group_form");
        run.click("new-group-create");
        run.until("the group", |r| {
            let id = r.current();
            id.is_some_and(|id| {
                r.store(|s| s.context(&id).is_some_and(|c| c.title == "Weekend trip"))
            })
        });
        run.type_into("composer-input", "Who brings the tent?");
        run.click("composer-send");
        run.settle();
        run.shot("f14_10_group_created");

        fake.shutdown();
    }
}
