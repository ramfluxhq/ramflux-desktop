// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Renders reference screenshots of the production window root with GPUI's
//! offscreen renderer (Metal on macOS, WGPU on Linux), in light and dark.
//!
//! Not part of `cargo test`; run explicitly:
//!
//! ```sh
//! RAMFLUX_SHOTS_DIR=/some/dir cargo test -p ramflux-desktop-ui --test screenshots
//! ```
//!
//! `RAMFLUX_SHOTS_SET=hardening` renders only the loading and empty states
//! and the keyboard focus states (shots 10 to 14);
//! `RAMFLUX_SHOTS_SET=conformance` renders the renderer conformance states
//! (shots 15 to 17); `RAMFLUX_SHOTS_SET=ai_participant` renders the AI
//! participant in the conversation (shots 18 to 22).
//!
//! The developer panel stays closed: scenarios are loaded and time is moved
//! through the store handle (as the panel would), and every interaction is
//! real input on the production views.

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

    use gpui_kit::base::Root;
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
    use gpui_kit::{
        AppContext as _, Bounds, Entity, HeadlessAppContext, Point, WindowBounds, WindowHandle,
        WindowOptions, px, size,
    };
    use ramflux_desktop_core::{
        Account, AccountId, AccountKind, ContextId, DomainEvent, Listing, LockState, MomentId,
        SourceStatus, Timestamp,
    };
    use ramflux_desktop_test::{Scenario, load_conformance, load_fixture};
    use ramflux_desktop_ui::AppAssets as Assets;
    use ramflux_desktop_ui::Shell;
    use ramflux_desktop_ui::actions::OpenPalette;
    use ramflux_desktop_ui::settings::SettingsSection;

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
                        Bounds { origin: Point::default(), size: size(px(1440.), px(960.)) };
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

        fn click_in(&mut self, scope: &'static str, id: &'static str) {
            self.in_window(|window, cx| window.within(scope).click(id, cx));
        }

        fn type_into(&mut self, id: &'static str, text: &'static str) {
            self.in_window(|window, cx| {
                window.click(id, cx);
                window.input(text, cx);
            });
        }

        /// Plays a bundled scenario from the start (what the dev panel's
        /// scenario buttons do) and seeks to `ms`.
        fn scenario(&mut self, name: &str, ms: u64) {
            let scenario = load_fixture(name).expect("fixture");
            self.play(&scenario, ms);
        }

        fn play(&mut self, scenario: &Scenario, ms: u64) {
            let store = self.cx.update(|cx| self.shell.read(cx).store().clone());
            self.cx.update(|cx| {
                store.update(cx, |store, cx| {
                    store.load_scenario(scenario, cx);
                    // Advance (not seek) so background accounts move too.
                    store.advance(ms, cx);
                });
            });
            self.cx.run_until_parked();
        }

        /// Moves scenario time forward (responses to submitted effects).
        fn advance(&mut self, ms: u64) {
            let store = self.cx.update(|cx| self.shell.read(cx).store().clone());
            self.cx.update(|cx| store.update(cx, |store, cx| store.advance(ms, cx)));
            self.cx.run_until_parked();
        }

        fn show(&mut self, context: &str, moment: Option<&str>) {
            let workspace = self.cx.update(|cx| self.shell.read(cx).workspace().clone());
            let context = ContextId::new(context);
            let moment = moment.map(MomentId::new);
            self.cx
                .update_window(self.handle.into(), |_, window, cx| {
                    workspace.update(cx, |w, cx| {
                        w.select_context(Some(context), window, cx);
                        if let Some(moment) = &moment {
                            w.conversation().update(cx, |field, cx| field.focus_moment(moment, cx));
                        }
                    });
                })
                .expect("window");
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
            let dev_open = self.cx.update(|cx| self.shell.read(cx).is_dev_panel_visible());
            assert!(!dev_open, "reference screenshots are taken with the developer panel closed");
            self.in_window(|_, _| {});
            let image = self.cx.capture_screenshot(self.handle.into()).expect("offscreen render");
            let path = self.out.join(name);
            image.save(&path).expect("write png");
            println!("wrote {}", path.display());
        }

        /// Lets value transitions (progress bars) finish before a capture.
        fn settle(&mut self) {
            for _ in 0..5 {
                self.cx.advance_clock(std::time::Duration::from_millis(200));
                self.in_window(|_, _| {});
            }
        }

        /// Saves `name` in light and dark.
        fn both(&mut self, name: &str) {
            self.theme(ThemeMode::Light);
            self.save(&format!("{name}_light.png"));
            self.theme(ThemeMode::Dark);
            self.save(&format!("{name}_dark.png"));
        }
    }

    pub fn run() {
        let out = std::env::var_os("RAMFLUX_SHOTS_DIR").map_or_else(
            || PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("shots"),
            PathBuf::from,
        );
        std::fs::create_dir_all(&out).expect("output directory");
        let mut s = Session::open(out);
        match std::env::var("RAMFLUX_SHOTS_SET").as_deref() {
            Ok("hardening") => return hardening(&mut s),
            Ok("conformance") => return conformance(&mut s),
            Ok("ai_participant") => return ai_participant(&mut s),
            _ => {}
        }

        // 1. Entry: a node added by address, its NODE-01 card with the
        //    trust-on-first-use warning, and the hold strip for trusting it.
        s.scenario("s01_first_launch", 3_000);
        s.click_in("node-n-harbor", "node-confirm");
        s.advance(200);
        s.click("entry-review-nodes");
        s.type_into("node-url", "https://studio.example.org");
        s.click("node-add");
        s.advance(400);
        s.click_in("node-n-studio", "node-confirm");
        s.both("01_entry_tofu_node");

        // 2. Workspace: an agent reply streaming into its surface.
        s.scenario("s02_daily", 800);
        s.show("c-orbit", None);
        s.both("02_workspace_streaming");

        // 3. Task progress: the step that needs the user, expanded with its decision.
        s.scenario("s06_in_progress", 200);
        s.click("composer-mode");
        s.type_into("composer-input", "Book a train to Lyon on Friday morning");
        s.click("composer-send");
        s.advance(3_000);
        s.click_in("moment-m-task", "task-steps");
        s.both("03_task_needs_me");

        // 4. Conflict: side-by-side compare and the three resolutions.
        s.scenario("s07_conflict_privacy_handoff", 1_000);
        s.show("c-ledger", Some("m-conflict"));
        s.both("04_conflict_compare");

        // 5. Approvals dock: pending, grants, standing approvals, audit.
        s.scenario("s03_intervention", 1_000);
        s.click("rail-approvals");
        s.both("05_approvals_dock");
        s.click("rail-approvals");

        // 6. Command palette open over the workspace.
        s.scenario("s02_daily", 3_000);
        s.in_window(|window, cx| window.dispatch_action(Box::new(OpenPalette), cx));
        s.both("06_command_palette");
        s.in_window(|window, cx| {
            window.dispatch_action(Box::new(ramflux_desktop_ui::actions::ClosePalette), cx)
        });

        // 7. Settings · Nodes: profiles, probe results and actions.
        s.scenario("s09_multi_account_day", 2_000);
        let shell = s.shell.clone();
        s.cx.update(|cx| {
            shell.update(cx, |shell, cx| shell.open_settings_at(SettingsSection::Nodes, cx))
        });
        s.cx.run_until_parked();
        s.both("07_settings_nodes");
        s.cx.update(|cx| shell.update(cx, |shell, cx| shell.close_settings_now(cx)));

        // 8. Contacts dock beside a sticky safety warning.
        s.scenario("s04_safety", 1_000);
        s.show("c-mira", None);
        s.click("rail-contacts");
        s.both("08_contacts_dock");
        s.click("rail-contacts");

        // 9. Objects dock: transfers in a table with their actions.
        s.scenario("s02_daily", 3_000);
        s.click("rail-objects");
        s.both("09_objects_dock");
    }

    /// An unlocked account with no conversation yet, listed at 500 ms.
    fn empty_account() -> Scenario {
        let account = Account {
            id: AccountId::new("a-new"),
            label: "Personal".to_owned(),
            kind: AccountKind::HumanInteractive,
            node_id: None,
            device_id: None,
            lock_state: LockState::Unlocked,
            self_participant_id: None,
            secure_sessions: ramflux_desktop_core::SecureSessions::Ready,
        };
        Scenario {
            name: "empty_account".to_owned(),
            description: "A new account whose lists are empty".to_owned(),
            epoch: Timestamp(1_790_812_800_000),
            timeline: vec![
                (0, DomainEvent::SourceStatusChanged(SourceStatus::Connected)),
                (0, DomainEvent::AccountUpdated(account)),
                (500, DomainEvent::ListingsLoaded(Listing::ALL.to_vec())),
            ],
            responses: Vec::new(),
            companions: Vec::new(),
        }
    }

    /// Loading and empty states, and keyboard focus.
    fn hardening(s: &mut Session) {
        // 10. Entry: the daemon answered, nodes not listed yet (loading).
        s.scenario("s01_first_launch", 1_600);
        s.both("10_entry_nodes_loading");

        // 11. Workspace of a new account: no conversations yet (empty, not loading).
        s.play(&empty_account(), 600);
        s.both("11_workspace_no_conversations");

        // 12. The contacts dock of that account: listed and empty.
        s.click("rail-contacts");
        s.both("12_contacts_dock_empty");
        s.click("rail-contacts");

        // 13. Keyboard: secondary-j focuses the moment that needs the user,
        //     Tab moves to its Approve button (focus ring).
        s.scenario("s03_intervention", 1_000);
        s.in_window(|window, cx| window.press("secondary-j", cx));
        for _ in 0..6 {
            let on_approve = std::cell::Cell::new(false);
            s.in_window(|window, _| {
                let found = window.within("moment-m-approve").try_find("approve");
                on_approve.set(found.and_then(|e| e.focused()) == Some(true));
            });
            if on_approve.get() {
                break;
            }
            s.in_window(|window, cx| window.press("tab", cx));
        }
        s.both("13_keyboard_focus_approve");

        // 14. Enter opens the hold strip with focus on the hold control;
        //     Space held part-way shows the progress.
        s.in_window(|window, cx| window.press("enter", cx));
        s.in_window(|window, cx| window.key_down("space", false, cx));
        s.cx.advance_clock(std::time::Duration::from_millis(500));
        s.cx.run_until_parked();
        s.both("14_keyboard_hold_in_progress");
        s.in_window(|window, cx| window.key_up("space", cx));
    }

    /// Renderer conformance states.
    fn conformance(s: &mut Session) {
        // 15. A patch lands while the hold strip is held: the hold is
        //     cancelled and the surface asks for a new review.
        let l2 = load_conformance("l2_interactive").expect("fixture");
        s.play(&l2, 2_500);
        s.show("c-conf", Some("m-form"));
        s.click_in("moment-m-form", "action-deploy");
        let center = std::cell::Cell::new(None);
        s.in_window(|window, cx| {
            let at = window.find("hold-confirm").bounds().center();
            window.pointer_down(at, gpui_kit::MouseButton::Left, cx);
            center.set(Some(at));
        });
        s.cx.advance_clock(std::time::Duration::from_millis(400));
        s.advance(600);
        if let Some(at) = center.get() {
            s.in_window(|window, cx| window.pointer_up(at, gpui_kit::MouseButton::Left, cx));
        }
        s.show("c-conf", Some("m-form"));
        s.both("15_surface_changed_review_again");

        // 16. A surface over the action limit: only its summary, no actions.
        let l1 = load_conformance("l1_documents").expect("fixture");
        s.play(&l1, 1_100);
        s.show("c-conf", Some("m-over"));
        s.both("16_surface_over_limit_fallback");

        // 17. A high-risk approval refused by the backend: fail-closed
        //     inline and in the approvals dock, never retried.
        s.play(&l2, 500);
        s.show("c-conf", Some("m-tool"));
        s.click_in("moment-m-tool", "approve");
        s.in_window(|window, cx| {
            let at = window.find("hold-confirm").bounds().center();
            window.pointer_down(at, gpui_kit::MouseButton::Left, cx);
            center.set(Some(at));
        });
        s.cx.advance_clock(std::time::Duration::from_millis(1_400));
        s.cx.run_until_parked();
        if let Some(at) = center.get() {
            s.in_window(|window, cx| window.pointer_up(at, gpui_kit::MouseButton::Left, cx));
        }
        s.advance(300);
        s.show("c-conf", Some("m-tool"));
        s.click("rail-approvals");
        s.both("17_grant_fail_closed");
        s.click("rail-approvals");
    }

    /// The AI participant in the conversation (demo 3 and surface versions).
    fn ai_participant(s: &mut Session) {
        // 18. Orbit's reply streaming into its surface, mid-way.
        s.scenario("s10_ai_participant", 200);
        s.show("c-launch", None);
        s.type_into("composer-input", "@Orbit can you check the launch checklist?");
        s.click("composer-send");
        s.advance(750);
        s.settle();
        s.both("18_ai_reply_streaming");

        // 19. The task Orbit proposed, started; its deploy step needs me.
        s.advance(400);
        s.click_in("moment-m-task", "task-start");
        s.advance(1_400);
        s.show("c-launch", Some("m-task"));
        s.settle();
        s.both("19_ai_task_needs_me");

        // 20. Approve pressed: the hold-to-confirm strip above the composer.
        s.click_in("moment-m-task", "step-approve");
        s.settle();
        s.both("20_ai_approval_confirm");

        // 22. Held: the result patched into Orbit's checklist, task done.
        let center = std::cell::Cell::new(None);
        s.in_window(|window, cx| {
            let at = window.find("hold-confirm").bounds().center();
            window.pointer_down(at, gpui_kit::MouseButton::Left, cx);
            center.set(Some(at));
        });
        s.cx.advance_clock(std::time::Duration::from_millis(1_400));
        s.cx.run_until_parked();
        if let Some(at) = center.get() {
            s.in_window(|window, cx| window.pointer_up(at, gpui_kit::MouseButton::Left, cx));
        }
        s.advance(1_700);
        s.show("c-launch", Some("m-task"));
        s.settle();
        s.both("22_ai_task_done");

        // 21. A surface whose change was made against an older version.
        s.scenario("s11_surface_versions", 2_600);
        s.show("c-plan", Some("m-pick"));
        s.settle();
        s.both("21_ai_surface_conflict");
    }
}
