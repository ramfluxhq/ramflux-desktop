// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Offscreen screenshots of the production shell at the design reference
//! size (1440×900), one per reviewed design board, in Simplified Chinese
//! and English, light and dark, with the developer panel closed.
//!
//! Not part of `cargo test`; run explicitly:
//!
//! ```sh
//! RAMFLUX_SHOTS_DIR=/some/dir cargo test -p ramflux-desktop-ui --test design_align_shots
//! ```

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    shots::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("design align shots: skipped; GPUI has no headless renderer on this platform");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod shots {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use gpui_kit::base::Root;
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Bounds, Entity, HeadlessAppContext, Point, Window, WindowBounds,
        WindowHandle, WindowOptions, px, size,
    };
    use ramflux_desktop_core::{
        ContactRequest, ContextId, DomainEvent, ParticipantId, RequestDirection, RequestState,
    };
    use ramflux_desktop_test::load_fixture;
    use ramflux_desktop_ui::AppAssets as Assets;
    use ramflux_desktop_ui::Shell;
    use ramflux_desktop_ui::actions::{ClosePalette, OpenPalette};
    use ramflux_desktop_ui::i18n;
    use ramflux_desktop_ui::settings::SettingsSection;
    use ramflux_desktop_ui::workspace::WorkspaceRoute;

    /// What a shot does after each language and theme switch.
    type Prepare = dyn Fn(&mut Window, &mut gpui_kit::App);

    struct Run {
        cx: HeadlessAppContext,
        handle: WindowHandle<Root>,
        shell: Entity<Shell>,
        out: PathBuf,
    }

    impl Run {
        fn open(scenario: &str, out: &Path) -> Self {
            let mut cx = HeadlessAppContext::with_platform(
                gpui_kit::platform::current_platform(true).text_system(),
                Arc::new(Assets),
                gpui_kit::platform::current_headless_renderer,
            );
            cx.update(|cx| {
                gpui_kit::init(cx);
                ramflux_desktop_ui::init(cx);
                ramflux_desktop_ui::assets::load_fonts(cx).expect("bundled fonts");
                Theme::change(ThemeMode::Light, None, cx);
            });
            let name = scenario.to_owned();
            let (handle, shell) = cx
                .update(|cx| {
                    let bounds =
                        Bounds { origin: Point::default(), size: size(px(1440.), px(900.)) };
                    gpui_kit::open_window(
                        WindowOptions {
                            window_bounds: Some(WindowBounds::Windowed(bounds)),
                            focus: false,
                            show: false,
                            ..Default::default()
                        },
                        cx,
                        move |window, cx| {
                            cx.new(|cx| Shell::with_scenario(&name, false, window, cx))
                        },
                    )
                })
                .expect("offscreen window");
            let handle = handle.downcast::<Root>().expect("Root");
            Self { cx, handle, shell, out: out.to_path_buf() }
        }

        fn play(&mut self, scenario: &ramflux_desktop_test::Scenario, ms: u64) {
            let store = self.cx.update(|cx| self.shell.read(cx).store().clone());
            self.cx.update(|cx| {
                store.update(cx, |store, cx| {
                    store.load_scenario(scenario, cx);
                    store.advance(ms, cx);
                })
            });
            self.cx.run_until_parked();
        }

        fn advance(&mut self, ms: u64) {
            let store = self.cx.update(|cx| self.shell.read(cx).store().clone());
            self.cx.update(|cx| store.update(cx, |store, cx| store.advance(ms, cx)));
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

        fn route(&mut self, route: WorkspaceRoute) {
            let workspace = self.cx.update(|cx| self.shell.read(cx).workspace().clone());
            self.window(move |window, cx| {
                workspace.update(cx, |w, cx| w.navigate(route, window, cx));
            });
        }

        fn settings(&mut self, section: SettingsSection) {
            let shell = self.shell.clone();
            self.cx.update(|cx| shell.update(cx, |s, cx| s.open_settings_at(section, cx)));
            self.cx.run_until_parked();
        }

        fn close_settings(&mut self) {
            let shell = self.shell.clone();
            self.cx.update(|cx| shell.update(cx, |s, cx| s.close_settings_now(cx)));
            self.cx.run_until_parked();
        }

        /// Saves `name` in Chinese and English, light and dark, after
        /// checking that every landmark of the board is on screen.
        /// `prepare` runs after each switch (to open a popover, for one).
        fn shot_with(&mut self, name: &str, landmarks: &[&str], prepare: Option<&Prepare>) {
            let dev = self.cx.update(|cx| self.shell.read(cx).is_dev_panel_visible());
            assert!(!dev, "screenshots are taken with the developer panel closed");
            for (locale, lang) in [("zh-CN", "zh"), ("en", "en")] {
                for (mode, suffix) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
                    self.cx.update(|cx| {
                        i18n::select_locale(locale, cx);
                        Theme::change(mode, None, cx);
                        cx.refresh_windows();
                    });
                    if let Some(prepare) = prepare {
                        self.window(|window, cx| prepare(window, cx));
                    }
                    self.window(|_, _| {});
                    for id in landmarks {
                        let id = (*id).to_owned();
                        let shown = id.clone();
                        let found = self.window(move |window, _| {
                            window.try_find(gpui_kit::SharedString::from(id)).is_some()
                        });
                        assert!(found, "{name} ({lang}, {suffix}): landmark {shown} is missing");
                    }
                    let image = self.cx.capture_screenshot(self.handle.into()).expect("render");
                    let path = self.out.join(format!("{name}_{lang}_{suffix}.png"));
                    image.save(&path).expect("write png");
                    println!("wrote {}", path.display());
                    // An opened popover or palette is dismissed again.
                    if prepare.is_some() {
                        self.window(|window, cx| window.press("escape", cx));
                    }
                }
            }
            self.cx.update(|cx| {
                i18n::select_locale(i18n::DEFAULT_LOCALE, cx);
                Theme::change(ThemeMode::Light, None, cx);
                cx.refresh_windows();
            });
            self.cx.run_until_parked();
        }

        fn shot(&mut self, name: &str, landmarks: &[&str]) {
            self.shot_with(name, landmarks, None);
        }
    }

    /// s02 with a pending stranger, so the strangers boards have content.
    fn with_stranger() -> ramflux_desktop_test::Scenario {
        let mut scenario = load_fixture("s02_daily").expect("fixture");
        let request = ContactRequest {
            id: "req-guzhou".into(),
            peer: ParticipantId::new("p-guzhou"),
            direction: RequestDirection::Incoming,
            state: RequestState::Pending,
            note: Some("Hello, could you share last week's review notes?".into()),
            safety_number: Some("73051186442090573312".into()),
            held_messages: 1,
            preview: None,
            expires_at: None,
        };
        scenario.timeline.push((10, DomainEvent::ContactRequestUpserted(request)));
        scenario.timeline.sort_by_key(|(at, _)| *at);
        scenario
    }

    pub fn run() {
        let out = std::env::var_os("RAMFLUX_SHOTS_DIR").map_or_else(
            || PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("design-align-shots"),
            PathBuf::from,
        );
        std::fs::create_dir_all(&out).expect("output directory");

        // Board 02 / I24: messages, a conversation open.
        let mut run = Run::open("s02_daily", &out);
        run.play(&with_stranger(), 3_000);
        run.show("c-team");
        run.shot(
            "02_messages",
            &[
                "navigation-rail",
                "stack-filters",
                "strangers-entry",
                "conversation-header",
                "composer",
            ],
        );
        // Board H01: the files tool open beside the conversation.
        run.window(|window, cx| window.click("rail-objects", cx));
        run.shot("h01_dock_files", &["dock-panel-objects", "dock-rail", "objects-cards"]);
        run.window(|window, cx| window.click("rail-objects", cx));
        // Board B09 / I11: command palette.
        run.shot_with(
            "b09_palette",
            &["palette"],
            Some(&|window: &mut Window, cx: &mut gpui_kit::App| {
                window.dispatch_action(Box::new(OpenPalette), cx)
            }),
        );
        run.window(|window, cx| window.dispatch_action(Box::new(ClosePalette), cx));
        // Board 04: the Agents section, the private conversation with Orbit.
        run.route(WorkspaceRoute::Agents);
        run.window(|window, cx| window.click("agent-p-orbit", cx));
        run.shot(
            "04_agent_conversation",
            &["agent-roster", "conversation-agent-profile", "composer-receiver"],
        );
        // Board 03 / C05: groups.
        run.route(WorkspaceRoute::Groups);
        run.shot("03_groups", &["groups-search-input"]);
        // Board 07: contacts (people, strangers card, table).
        run.route(WorkspaceRoute::Contacts);
        run.shot("07_contacts", &["contacts-people", "contacts-strangers-table", "contacts-table"]);
        // Board F17 / I26: strangers, one request chosen.
        run.route(WorkspaceRoute::Strangers);
        let workspace = run.cx.update(|cx| run.shell.read(cx).workspace().clone());
        let area = run.cx.update(|cx| workspace.read(cx).strangers().clone());
        run.cx.update(|cx| area.update(cx, |a, cx| a.select(Some("req-guzhou".into()), cx)));
        run.cx.run_until_parked();
        run.shot("f17_strangers", &["strangers-area", "stranger-accept"]);
        // Boards G01, G13 (and the other settings sections).
        run.route(WorkspaceRoute::Messages);
        for (name, section, landmark) in [
            ("g01_settings_account", SettingsSection::Account, "account-current"),
            ("g13_settings_appearance", SettingsSection::Appearance, "appearance-mode"),
            ("d01_settings_agents", SettingsSection::Agents, "settings-header"),
        ] {
            run.settings(section);
            run.shot(name, &["settings-header", landmark]);
            run.close_settings();
        }

        // Boards G08 / G09: devices and nodes, listed.
        let mut ops = Run::open("s03_intervention", &out);
        ops.advance(1_000);
        ops.settings(SettingsSection::Devices);
        ops.shot("g08_settings_devices", &["settings-header"]);
        ops.close_settings();
        let mut nodes = Run::open("s09_multi_account_day", &out);
        nodes.advance(2_000);
        nodes.settings(SettingsSection::Nodes);
        nodes.shot("g09_settings_nodes", &["settings-header"]);
        nodes.close_settings();
        // Board B01: the identity popover, two accounts (Current / Switch).
        nodes.shot_with(
            "b01_account_menu",
            &["identity-menu", "identity-switch-a-work"],
            Some(&|window: &mut Window, cx: &mut gpui_kit::App| window.click("nav-account", cx)),
        );

        // Board 06: tasks, a proposal waiting for the user.
        let mut tasks = Run::open("s10_ai_participant", &out);
        tasks.advance(200);
        tasks.show("c-launch");
        tasks.window(|window, cx| {
            window.click("composer-input", cx);
            window.input("@Orbit can you check the launch checklist?", cx);
        });
        tasks.window(|window, cx| window.click("composer-send", cx));
        tasks.advance(1_100);
        tasks.route(WorkspaceRoute::Tasks);
        tasks.shot("06_tasks", &["tasks-area", "tasks-category-needs_me", "task-card-t-launch"]);

        // Board A01: first run, the local service step.
        let mut entry = Run::open("s01_first_launch", &out);
        entry.advance(100);
        entry.shot("a01_entry", &["entry-header", "entry-steps"]);
    }
}
