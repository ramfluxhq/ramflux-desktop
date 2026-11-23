// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! One offscreen window of the production shell over the mock source.
//!
//! Every capture opens a fresh headless application and window at the
//! capture size (the headless platform does not re-lay out a resized
//! window), applies the theme and locale the way the Appearance page does,
//! runs the board's recipe and renders one frame. Time is the mock's
//! scenario clock (or the test dispatcher's clock for the entry flows), never
//! the wall clock.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::Root;
use gpui_kit::component::ThemeMode;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Entity, HeadlessAppContext, Point, Window, WindowBounds, WindowHandle,
    WindowOptions, px, size,
};
use ramflux_desktop_core::{ContextId, DomainEvent};
use ramflux_desktop_test::Scenario;
use ramflux_desktop_ui::AppAssets as Assets;
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::settings::SettingsSection;
use ramflux_desktop_ui::workspace::WorkspaceRoute;

pub type Step = Result<(), String>;

/// How the shell is started.
#[derive(Clone, Copy)]
pub enum Start {
    /// A fixture scenario on the scenario clock (`Shell::with_scenario(_, false)`).
    Scenario(&'static str),
    /// A fixture scenario with its clock running on the test dispatcher
    /// (`Shell::with_scenario(_, true)`); time moves with [`Run::tick`].
    Live(&'static str),
}

pub struct Run {
    pub cx: HeadlessAppContext,
    pub handle: WindowHandle<Root>,
    pub shell: Entity<Shell>,
}

/// The `'static` tag of a shipped locale, if the application ships it.
pub fn shipped_locale(tag: &str) -> Option<&'static str> {
    ramflux_desktop_ui::i18n::LOCALES.iter().map(|(t, _)| *t).find(|t| *t == tag)
}

impl Run {
    /// Opens the window at `width` x `height` logical pixels with `theme` and
    /// `locale` applied.
    pub fn open(
        start: Start,
        width: u32,
        height: u32,
        theme: &str,
        locale: &'static str,
    ) -> Result<Self, String> {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        let fonts = cx.update(|cx| {
            gpui_kit::init(cx);
            ramflux_desktop_ui::init(cx);
            ramflux_desktop_ui::assets::load_fonts(cx)
        });
        fonts.map_err(|e| format!("bundled fonts: {e}"))?;
        let (name, live) = match start {
            Start::Scenario(name) => (name, false),
            Start::Live(name) => (name, true),
        };
        let opened = cx.update(|cx| {
            let bounds = Bounds {
                origin: Point::default(),
                size: size(px(width as f32), px(height as f32)),
            };
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                cx,
                move |window, cx| cx.new(|cx| Shell::with_scenario(name, live, window, cx)),
            )
        });
        let (handle, shell) = opened.map_err(|e| format!("offscreen window: {e}"))?;
        let handle = handle.downcast::<Root>().ok_or("the window root is not a Root")?;
        let mut run = Self { cx, handle, shell };
        run.apply(theme, locale);
        if live {
            // A person looks at an active window (the ceremony covers its
            // words in an inactive one).
            run.cx
                .update_window(run.handle.into(), |_, window, _| window.activate_window())
                .map_err(|e| format!("window: {e}"))?;
            run.cx.run_until_parked();
        }
        Ok(run)
    }

    /// Picks the theme mode and locale the way the Appearance page does.
    pub fn apply(&mut self, theme: &str, locale: &'static str) {
        use ramflux_desktop_navigation::theme;
        use ramflux_desktop_ui::preferences::{self, LanguageChoice, ThemeChoice};
        let mode = if theme == "dark" { ThemeMode::Dark } else { ThemeMode::Light };
        self.cx.update(|cx| {
            theme::select_mode(mode, cx);
            if let Some(info) = theme::active(cx) {
                preferences::set_theme(ThemeChoice::Named(info.id), cx);
            }
            preferences::set_language(LanguageChoice::Locale(locale), cx);
            cx.refresh_windows();
        });
        self.cx.run_until_parked();
    }

    /// Loads `scenario` into the mock and advances its clock by `ms`.
    pub fn play(&mut self, scenario: &Scenario, ms: u64) {
        let store = self.cx.update(|cx| self.shell.read(cx).store().clone());
        self.cx.update(|cx| {
            store.update(cx, |store, cx| {
                store.load_scenario(scenario, cx);
                store.advance(ms, cx);
            })
        });
        self.cx.run_until_parked();
    }

    /// Advances the scenario clock by `ms`.
    pub fn advance(&mut self, ms: u64) {
        let store = self.cx.update(|cx| self.shell.read(cx).store().clone());
        self.cx.update(|cx| store.update(cx, |store, cx| store.advance(ms, cx)));
        self.cx.run_until_parked();
    }

    /// Advances the dispatcher clock in 100 ms steps (live scenarios).
    pub fn tick(&mut self, ms: u64) {
        let mut left = ms;
        while left > 0 {
            let step = left.min(100);
            self.cx.advance_clock(Duration::from_millis(step));
            self.cx.run_until_parked();
            left -= step;
        }
    }

    /// Injects an event into the running mock after `delay_ms`.
    pub fn inject(&mut self, delay_ms: u64, event: DomainEvent) -> Step {
        let store = self.cx.update(|cx| self.shell.read(cx).store().clone());
        self.cx.update(|cx| {
            store.update(cx, |store, _| match store.mock_mut() {
                Some(mock) => {
                    mock.inject(delay_ms, event);
                    Ok(())
                }
                None => Err("the shell has no mock source".to_owned()),
            })
        })
    }

    /// Runs `f` on the mock source.
    pub fn mock<R>(
        &mut self,
        f: impl FnOnce(&mut ramflux_desktop_test::MockSource) -> R,
    ) -> Result<R, String> {
        let store = self.cx.update(|cx| self.shell.read(cx).store().clone());
        self.cx.update(|cx| {
            store.update(cx, |store, _| store.mock_mut().map(f).ok_or("no mock source".to_owned()))
        })
    }

    /// Renders a frame and runs `f` in the window.
    pub fn window<R>(
        &mut self,
        f: impl FnOnce(&mut Window, &mut gpui_kit::App) -> R,
    ) -> Result<R, String> {
        let result = self
            .cx
            .update_window(self.handle.into(), |_, window, cx| {
                window.render_frame(cx);
                f(window, cx)
            })
            .map_err(|e| format!("window: {e}"))?;
        self.cx.run_until_parked();
        Ok(result)
    }

    /// Whether the element `id` is painted.
    pub fn has(&mut self, id: &str) -> bool {
        let id = gpui_kit::SharedString::from(id.to_owned());
        self.window(move |window, _| window.try_find(id).is_some()).unwrap_or(false)
    }

    /// Clicks `id`, failing (instead of panicking) when it is not painted.
    pub fn click(&mut self, id: &'static str) -> Step {
        if !self.has(id) {
            return Err(format!("element {id} is not on screen"));
        }
        self.window(move |window, cx| window.click(id, cx))
    }

    /// Hovers `id` when it is painted.
    pub fn hover(&mut self, id: &'static str) -> Step {
        if !self.has(id) {
            return Err(format!("element {id} is not on screen"));
        }
        self.window(move |window, cx| window.hover(id, cx))
    }

    /// Clicks `id` and types `text` into it.
    pub fn type_into(&mut self, id: &'static str, text: &str) -> Step {
        self.click(id)?;
        let text = text.to_owned();
        self.window(move |window, cx| window.input(&text, cx))
    }

    pub fn dispatch(&mut self, action: Box<dyn gpui_kit::Action>) -> Step {
        self.window(move |window, cx| window.dispatch_action(action, cx))
    }

    /// Shows the conversation `context`.
    pub fn show(&mut self, context: &str) -> Step {
        let workspace = self.cx.update(|cx| self.shell.read(cx).workspace().clone());
        let id = ContextId::new(context);
        self.window(move |window, cx| {
            workspace.update(cx, |w, cx| w.select_context(Some(id), window, cx));
        })
    }

    /// Goes to a workspace section.
    pub fn route(&mut self, route: WorkspaceRoute) -> Step {
        let workspace = self.cx.update(|cx| self.shell.read(cx).workspace().clone());
        self.window(move |window, cx| {
            workspace.update(cx, |w, cx| w.navigate(route, window, cx));
        })
    }

    /// Runs `f` on the workspace with the window.
    pub fn workspace(
        &mut self,
        f: impl FnOnce(
            &mut ramflux_desktop_ui::workspace::Workspace,
            &mut Window,
            &mut gpui_kit::Context<ramflux_desktop_ui::workspace::Workspace>,
        ) + 'static,
    ) -> Step {
        let workspace = self.cx.update(|cx| self.shell.read(cx).workspace().clone());
        self.window(move |window, cx| workspace.update(cx, |w, cx| f(w, window, cx)))
    }

    /// Opens the settings overlay at `section`.
    pub fn settings(&mut self, section: SettingsSection) {
        let shell = self.shell.clone();
        self.cx.update(|cx| shell.update(cx, |s, cx| s.open_settings_at(section, cx)));
        self.cx.run_until_parked();
    }

    /// No focus ring or hover the board does not draw: the pointer rests on
    /// the rail's empty middle (or the entry header).
    pub fn rest(&mut self) -> Step {
        let target = if self.has("navigation-rail") {
            Some("navigation-rail")
        } else if self.has("entry-header") {
            Some("entry-header")
        } else {
            None
        };
        self.window(move |window, cx| {
            window.blur(cx);
            if let Some(target) = target {
                window.hover(target, cx);
            }
        })
    }

    pub fn dev_panel_visible(&mut self) -> bool {
        self.cx.update(|cx| self.shell.read(cx).is_dev_panel_visible())
    }

    /// Renders the window to an image.
    pub fn capture(&mut self) -> Result<image::RgbaImage, String> {
        self.window(|_, _| {})?;
        self.cx.capture_screenshot(self.handle.into()).map_err(|e| format!("render: {e}"))
    }
}
