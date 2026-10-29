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
    use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
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

        fn click_label(&mut self, label: &str) {
            let label = label.to_owned();
            self.window(move |window, cx| {
                let at = window.find_by_label(&label).bounds().center();
                window.pointer_down(at, gpui_kit::MouseButton::Left, cx);
                window.pointer_up(at, gpui_kit::MouseButton::Left, cx);
            });
        }

        /// Holds the hold-to-confirm control until it is done.
        fn hold_confirm(&mut self) {
            let at = self.window(|window, cx| {
                let at = window.find("hold-confirm").bounds().center();
                window.pointer_down(at, gpui_kit::MouseButton::Left, cx);
                at
            });
            for _ in 0..30 {
                self.tick();
            }
            self.window(move |window, cx| window.pointer_up(at, gpui_kit::MouseButton::Left, cx));
        }

        fn click(&mut self, id: &str) {
            let id = id.to_owned();
            self.window(move |window, cx| window.click(id, cx));
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
        floors_states(&out, &locale);
        group_states(&out, &locale);
        verified_kind_state(&out, &locale);
    }

    /// The bound on remembered requester identities: full with entries to
    /// forget (and the hold that asks before forgetting them), then full with
    /// nothing to forget yet.
    fn floors_states(out: &std::path::Path, locale: &str) {
        let (fake, state, socket, _tasks) =
            sdk_fake::start_with_tasks(&format!("desk-pin3-floors-{locale}"));
        let requests = state.lock().expect("state").requests.clone();
        let bus = Arc::new(fake);
        let weak = Arc::downgrade(&bus);
        requests.set_publisher(Box::new(move |a, t, b| {
            if let Some(bus) = weak.upgrade() {
                bus.publish(a, t, b);
            }
        }));
        let _ = requests.incoming("acct-main", "principal-mira", Some("Saw you at the meetup"));
        requests.set_floors(Some((4096, 37, 4096)));
        let mut run = Run::open(
            sdk_fake::options(&socket),
            out.to_path_buf(),
            locale.to_owned(),
            1280.,
            1100.,
        );
        run.until("the workspace", |r| {
            r.cx.update(|cx| r.shell.read(cx).page()) == Page::Workspace
        });
        run.click("rail-contacts");
        run.until("the notice", |r| r.exists("contacts-floors"));
        run.settle();
        run.shot("pin3_03_floors_full_clearable");
        run.click("contacts-floors-clear");
        run.until("the hold", |r| r.exists("hold-confirm"));
        run.settle();
        run.shot("pin3_04_floors_forget_hold");
        run.hold_confirm();
        run.until("the notice is gone", |r| !r.exists("contacts-floors"));
        requests.set_floors(Some((4096, 0, 4096)));
        bus.publish("acct-main", "contact.requested", serde_json::json!({}));
        run.until("the notice", |r| r.exists("contacts-floors"));
        run.settle();
        run.shot("pin3_05_floors_full_nothing_to_forget");
        if let Ok(bus) = Arc::try_unwrap(bus) {
            bus.shutdown();
        }
    }

    /// A group change refused while the group's head is held for an
    /// ownership transfer, for each reason the daemon gives.
    fn group_states(out: &std::path::Path, locale: &str) {
        let (fake, state, socket, _tasks) =
            sdk_fake::start_with_tasks(&format!("desk-pin3-group-{locale}"));
        state.lock().expect("state").groups = Some(vec![group_row()]);
        let mut run = Run::open(
            sdk_fake::options(&socket),
            out.to_path_buf(),
            locale.to_owned(),
            1280.,
            860.,
        );
        run.until("the workspace", |r| {
            r.cx.update(|cx| r.shell.read(cx).page()) == Page::Workspace
        });
        run.until("the group", |r| {
            r.store(|s| s.context(&ContextId::new(sdk_fake::GROUP)).is_some())
        });
        run.show(sdk_fake::GROUP);
        run.click("rail-members");
        run.until("the members", |r| r.exists("dock-content-members"));
        for (reason, name) in [
            ("head_check_pending", "pin3_06_group_sync_pending"),
            ("transfer_outcome_pending", "pin3_07_group_transfer_pending"),
            ("clock_skew_or_expired_control", "pin3_08_group_clock_skew"),
        ] {
            state.lock().expect("state").group_refusal = Some(reason);
            let before = run.store(|s| s.failures().len());
            run.window(|window, cx| {
                window.within("member-principal-bo").click("member-actions", cx)
            });
            run.click_label(&ramflux_desktop_ui::i18n::tr("dock.members.kick"));
            run.until("the hold", |r| r.exists("hold-confirm"));
            run.hold_confirm();
            run.until("the refusal", |r| r.store(|s| s.failures().len() > before));
            run.settle();
            run.shot(name);
        }
        fake.shutdown();
    }

    /// An event line whose kind the source verified names the kind.
    fn verified_kind_state(out: &std::path::Path, locale: &str) {
        let (fake, _state, socket, tasks) =
            sdk_fake::start_with_tasks(&format!("desk-pin3-kind-{locale}"));
        tasks.announce_messages(true);
        tasks.report_event_kinds(true);
        tasks.set_kind("principal-ana", "human");
        tasks.verify_kind("principal-ana");
        let mut run = Run::open(
            sdk_fake::options(&socket),
            out.to_path_buf(),
            locale.to_owned(),
            1280.,
            860.,
        );
        run.until("the workspace", |r| {
            r.cx.update(|cx| r.shell.read(cx).page()) == Page::Workspace
        });
        run.until("the group", |r| {
            r.store(|s| s.context(&ContextId::new(sdk_fake::GROUP)).is_some())
        });
        run.show(sdk_fake::GROUP);
        tasks
            .propose(sdk_fake::GROUP, "t-plan", "Draft the release notes", &[("a", "Draft")])
            .expect("proposal");
        tasks.accept_as("principal-ana", sdk_fake::GROUP, "t-plan").expect("accept");
        run.until("the kind mark", |r| r.exists("task-event-kind"));
        run.settle();
        run.shot("pin3_09_event_kind_verified_named");
        fake.shutdown();
    }

    fn group_row() -> serde_json::Value {
        let routes: Vec<serde_json::Value> =
            [("device-me", "principal-me"), ("device-bo", "principal-bo")]
                .iter()
                .map(|(device, principal)| {
                    serde_json::json!({ "member_id": device, "principal_id": principal,
                        "member_principal_commitment": format!("cm-{principal}"),
                        "device_signing_public_key": format!("key-{principal}"),
                        "target_delivery_id": format!("target-{principal}"),
                        "federation": null, "delivery_target_known": true })
                })
                .collect();
        serde_json::json!({ "group_id": "g-team", "id": "g-team", "group_epoch": 3, "name": "Team",
            "name_epoch": 3, "member_routes": routes,
            "conversation_id": sdk_fake::GROUP, "conversation_ids": [sdk_fake::GROUP] })
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
