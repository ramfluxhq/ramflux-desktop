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
    use ramflux_desktop_core::ContextId;
    use ramflux_desktop_ui::AppAssets as Assets;
    use ramflux_desktop_ui::Shell;
    use ramflux_desktop_ui::actions::{ClosePalette, OpenPalette};
    use ramflux_desktop_ui::i18n;
    use ramflux_desktop_ui::settings::SettingsSection;
    use ramflux_desktop_ui::workspace::WorkspaceRoute;

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

        /// Saves `name` in Chinese and English, light and dark.
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
                    let path = self.out.join(format!("{name}_{lang}_{suffix}.png"));
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

    pub fn run() {
        let out = std::env::var_os("RAMFLUX_SHOTS_DIR").map_or_else(
            || PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("design-align-shots"),
            PathBuf::from,
        );
        std::fs::create_dir_all(&out).expect("output directory");

        // Board 02 / I24: messages, a conversation open.
        let mut run = Run::open("s02_daily", &out);
        run.advance(3_000);
        run.show("c-team");
        run.shot("02_messages");
        // Board H01: the files tool open beside the conversation.
        run.window(|window, cx| window.click("rail-objects", cx));
        run.shot("h01_dock_files");
        run.window(|window, cx| window.click("rail-objects", cx));
        // Board B01: the identity menu of the rail's avatar.
        run.window(|window, cx| window.click("nav-account", cx));
        run.shot("b01_account_menu");
        run.window(|window, cx| window.press("escape", cx));
        // Board 02L-like: the direct conversation with an agent.
        run.show("c-orbit");
        run.shot("04_agent_conversation");
        // Board 03 / C05: groups.
        run.route(WorkspaceRoute::Groups);
        run.shot("03_groups");
        // Board 07: contacts.
        run.route(WorkspaceRoute::Contacts);
        run.shot("07_contacts");
        // Board F17 / I26: strangers.
        run.route(WorkspaceRoute::Strangers);
        run.shot("f17_strangers");
        // Board 06 / E01: tasks.
        run.route(WorkspaceRoute::Tasks);
        run.shot("06_tasks");
        run.route(WorkspaceRoute::Messages);
        // Board B09 / I11: command palette.
        run.window(|window, cx| window.dispatch_action(Box::new(OpenPalette), cx));
        run.shot("b09_palette");
        run.window(|window, cx| window.dispatch_action(Box::new(ClosePalette), cx));
        // Boards G01, G08, G09, G13, D01: settings.
        for (name, section) in [
            ("g01_settings_account", SettingsSection::Account),
            ("g08_settings_devices", SettingsSection::Devices),
            ("g09_settings_nodes", SettingsSection::Nodes),
            ("g13_settings_appearance", SettingsSection::Appearance),
            ("d01_settings_agents", SettingsSection::Agents),
        ] {
            run.settings(section);
            run.shot(name);
            run.close_settings();
        }

        // Board A01: first run, connecting.
        let mut entry = Run::open("s01_first_launch", &out);
        entry.advance(3_000);
        entry.shot("a01_entry");
    }
}
