// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Offscreen light and dark screenshots of the states DESK-PIN-01 adds, on
//! a fake daemon: a group's roster and signed name changed by sync, an
//! authenticated kind that does not fit what a member did, a call past its
//! control receipt cap, contact requests (a decline, a crossed request, the
//! cooldown explained in the add form), and a blocked contact that can be
//! blocked again after an unblock.
//!
//! Not part of `cargo test`; run explicitly (`RAMFLUX_SHOTS_LOCALE` picks
//! the interface language, `en` by default):
//!
//! ```sh
//! RAMFLUX_SHOTS_DIR=/some/dir RAMFLUX_SHOTS_LOCALE=zh-CN \
//!   cargo test -p ramflux-desktop-ui --test desk_pin_shots
//! ```

mod common;
mod sdk_fake;

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    shots::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("desk pin shots: skipped; GPUI has no headless renderer on this platform");
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
    use ramflux_desktop_core::{CallId, CallState, ContextId, ParticipantId};
    use ramflux_desktop_ui::AppAssets as Assets;
    use ramflux_desktop_ui::Shell;
    use ramflux_desktop_ui::route::Page;
    use ramflux_desktop_ui::shell::{ShellOptions, SourceChoice};
    use serde_json::json;

    use crate::sdk_fake;

    struct Run {
        cx: HeadlessAppContext,
        handle: WindowHandle<Root>,
        shell: Entity<Shell>,
        out: PathBuf,
        locale: String,
    }

    impl Run {
        fn open(options: ShellOptions, out: PathBuf, locale: String) -> Self {
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
                        Bounds { origin: Point::default(), size: size(px(1280.), px(860.)) };
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
            Self { cx, handle, shell, out, locale }
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

    fn group_row(epoch: u64, name: &str, members: &[(&str, &str)]) -> serde_json::Value {
        let routes: Vec<serde_json::Value> = members
            .iter()
            .map(|(device, principal)| {
                json!({ "member_id": device, "principal_id": principal,
                        "member_principal_commitment": format!("cm-{principal}"),
                        "device_signing_public_key": format!("key-{principal}"),
                        "target_delivery_id": format!("target-{principal}"),
                        "federation": null, "delivery_target_known": true })
            })
            .collect();
        json!({ "group_id": "g-team", "id": "g-team", "group_epoch": epoch, "name": name,
                "name_epoch": epoch, "member_routes": routes,
                "conversation_id": sdk_fake::GROUP, "conversation_ids": [sdk_fake::GROUP] })
    }

    fn out_dir() -> (PathBuf, String) {
        let out = std::env::var_os("RAMFLUX_SHOTS_DIR").map_or_else(
            || PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("desk-pin-shots"),
            PathBuf::from,
        );
        let locale = std::env::var("RAMFLUX_SHOTS_LOCALE").unwrap_or_else(|_| "en".to_owned());
        std::fs::create_dir_all(&out).expect("output directory");
        (out, locale)
    }

    pub fn run() {
        let (out, locale) = out_dir();
        daemon_states(out.clone(), locale.clone());
        blocked_states(out, locale);
    }

    /// Group sync, kind mismatch, call capacity and contact requests.
    fn daemon_states(out: PathBuf, locale: String) {
        let (fake, state, socket, tasks) =
            sdk_fake::start_with_tasks(&format!("desk-pin-shots-{locale}"));
        tasks.announce_messages(true);
        tasks.report_event_kinds(true);
        tasks.set_kind("principal-ana", "headless_ai");
        let requests = state.lock().expect("state").requests.clone();
        let calls = state.lock().expect("state").calls.clone();
        let bus = Arc::new(fake);
        let weak = Arc::downgrade(&bus);
        let publish = move |account: &str, topic: &str, body: serde_json::Value| {
            if let Some(bus) = weak.upgrade() {
                bus.publish(account, topic, body);
            }
        };
        let requests_publish = publish.clone();
        requests.set_publisher(Box::new(move |a, t, b| requests_publish(a, t, b)));
        let calls_publish = publish.clone();
        calls.set_publisher(Box::new(move |a, t, b| calls_publish(a, t, b)));
        state.lock().expect("state").groups = Some(vec![group_row(
            3,
            "Team",
            &[("device-me", "principal-me"), ("device-bo", "principal-bo")],
        )]);

        let mut options = sdk_fake::options(&socket);
        if let SourceChoice::Rfd(config) = &mut options.source {
            config.call_api = true;
        }
        let mut run = Run::open(options, out, locale);
        run.until("the workspace", |r| {
            r.cx.update(|cx| r.shell.read(cx).page()) == Page::Workspace
        });
        run.until("the group", |r| {
            r.store(|s| s.context(&ContextId::new(sdk_fake::GROUP)).is_some())
        });
        run.show(sdk_fake::GROUP);
        run.settle();

        // The creator renamed the group, added Ana and removed Bo.
        state.lock().expect("state").groups = Some(vec![group_row(
            4,
            "Release review",
            &[("device-me", "principal-me"), ("device-ana", "principal-ana")],
        )]);
        publish("acct-main", "group.changed", json!({ "group_id": "g-team" }));
        run.until("the sync records", |r| {
            r.exists("group-rename") && r.exists("governance-record")
        });
        run.settle();
        run.shot("pin_01_group_sync");

        // Ana's signed profile says she is an AI; she accepted a human task.
        tasks
            .propose(sdk_fake::GROUP, "t-plan", "Draft the release notes", &[("a", "Draft")])
            .expect("proposal");
        tasks.accept_as("principal-ana", sdk_fake::GROUP, "t-plan").expect("accept");
        run.until("the kind mark", |r| r.exists("task-event-kind"));
        run.settle();
        run.shot("pin_02_kind_mismatch");

        // A call past its control receipt cap.
        run.show("dm-ana");
        let id = CallId::new(calls.incoming("acct-main", "dm-ana"));
        run.until("ringing", |r| {
            r.store(|s| s.call(&id).map(|c| c.state)) == Some(CallState::IncomingRinging)
        });
        calls.set_refusal(Some((
            "CallControlReceiptCapacity",
            "voice call control receipt capacity exhausted at 128 receipts",
        )));
        run.click("incoming-answer");
        run.until("the refusal", |r| r.store(|s| !s.failures().is_empty()));
        run.settle();
        run.shot("pin_03_call_capacity");
        calls.set_refusal(None);

        // Contact requests: a crossed request, a decline and the cooldown.
        let own = requests.outgoing("acct-main", "principal-ana");
        let inbound = requests.incoming("acct-main", "principal-ana", Some("Saw your request too"));
        let _ = requests.outgoing("acct-main", "principal-bo");
        publish("acct-main", "contact.changed", json!({}));
        run.click("rail-contacts");
        run.until("both requests", |r| r.exists("contacts-request-principal-ana"));
        run.settle();
        run.shot("pin_04_crossed_request");
        run.click_in("contacts-request-principal-ana", "request-decline");
        run.until("declined", |_| requests.row(&inbound).is_some_and(|r| r["state"] == "declined"));
        run.settle();
        run.shot("pin_05_crossed_decline_own_waits");
        requests.peer_answers(&own, "declined");
        run.until("their decline", |r| {
            r.store(|s| {
                ramflux_desktop_core::selectors::outgoing_contact_request(
                    s,
                    &ParticipantId::new("principal-ana"),
                )
                .is_some_and(|q| q.state == ramflux_desktop_core::RequestState::Declined)
            })
        });
        run.type_into("contact-add-id", "principal-ana");
        run.click("contact-add");
        run.until("the cooldown", |r| r.exists("contact-add-note"));
        run.settle();
        run.shot("pin_06_declined_cooldown");
        if let Ok(bus) = Arc::try_unwrap(bus) {
            bus.shutdown();
        }
    }

    /// A blocked contact, and the same contact after an unblock (it can be
    /// blocked again).
    fn blocked_states(out: PathBuf, locale: String) {
        let options = ShellOptions::for_scenario("s04_safety", true);
        let mut run = Run::open(options, out, locale);
        run.until("the workspace", |r| {
            r.cx.update(|cx| r.shell.read(cx).page()) == Page::Workspace
        });
        run.settle();
        run.click("rail-contacts");
        run.until("the blocked contact", |r| r.exists("contacts-blocked-p-ivo"));
        run.settle();
        run.shot("pin_07_blocked_contact");
        run.click_in("contacts-blocked-p-ivo", "unblock");
        run.until("unblocked", |r| r.exists("contacts-p-ivo"));
        run.settle();
        run.shot("pin_08_unblocked_can_block_again");
    }
}
