// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The window root: title bar status strip, the page the store calls for
//! (entry or workspace), the settings overlay, and the developer panel.
//!
//! Window-wide commands are handled here and routed to the view that owns
//! them, so a command works the same from a key binding, the title bar, or a
//! menu.

use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, Render, Subscription, Window, div,
};
use ramflux_desktop_core::{AccountId, Intent, IntentKind, SecretInput};
use ramflux_desktop_test::load_fixture;

use crate::actions::{
    CloseSettings, GoToNeedsMe, OpenSettings, ToggleDevPanel, TogglePinContext, ToggleTheme,
};
use crate::app::{StoreEvent, StoreHandle};
use crate::dev::DevPanel;
use crate::entry::{EntryPage, UnlockHandler};
use crate::route::{Page, page_for};
use crate::settings::SettingsPage;
use crate::workspace::{StatusStrip, Workspace};

/// Scenario played when the application starts.
pub const DEFAULT_SCENARIO: &str = "s01_first_launch";

/// The window root view.
pub struct Shell {
    store: Entity<StoreHandle>,
    status: Entity<StatusStrip>,
    entry: Entity<EntryPage>,
    workspace: Entity<Workspace>,
    dev: Option<Entity<DevPanel>>,
    dev_visible: bool,
    settings_open: bool,
    page: Page,
    focus_handle: FocusHandle,
    _subscription: Subscription,
}

impl Focusable for Shell {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Shell {
    /// The application window root, playing [`DEFAULT_SCENARIO`] in real time.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_scenario(DEFAULT_SCENARIO, true, window, cx)
    }

    /// A root playing the bundled scenario `name`. With `start_clock` false
    /// time only moves through the developer controls (deterministic tests).
    pub fn with_scenario(
        name: &str,
        start_clock: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let store = cx.new(|cx| {
            let mut store = StoreHandle::new();
            if let Ok(scenario) = load_fixture(name) {
                store.load_scenario(&scenario, cx);
            }
            if start_clock {
                store.start_clock(cx);
            }
            store
        });
        let status = cx.new(|cx| StatusStrip::new(store.clone(), cx));
        let entry = cx.new(|cx| EntryPage::new(store.clone(), unlock_handler(&store), window, cx));
        let workspace = cx.new(|cx| Workspace::new(store.clone(), window, cx));
        let dev = cfg!(debug_assertions)
            .then(|| cx.new(|cx| DevPanel::new(store.clone(), workspace.downgrade(), cx)));
        let subscription = cx.subscribe_in(&store, window, |this, _, event: &StoreEvent, _, cx| {
            if let StoreEvent::Changed(changes) = event
                && changes.touches_status()
            {
                this.sync_page(cx);
            }
        });
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let mut shell = Self {
            store,
            status,
            entry,
            workspace,
            dev,
            dev_visible: false,
            settings_open: false,
            page: Page::Entry(crate::route::EntryStep::Daemon),
            focus_handle,
            _subscription: subscription,
        };
        shell.sync_page(cx);
        shell
    }

    /// The store handle.
    pub fn store(&self) -> &Entity<StoreHandle> {
        &self.store
    }

    /// The workspace page.
    pub fn workspace(&self) -> &Entity<Workspace> {
        &self.workspace
    }

    /// The entry page.
    pub fn entry(&self) -> &Entity<EntryPage> {
        &self.entry
    }

    /// The developer panel, in debug builds.
    pub fn dev_panel(&self) -> Option<&Entity<DevPanel>> {
        self.dev.as_ref()
    }

    /// The shown page.
    pub fn page(&self) -> Page {
        self.page
    }

    /// Whether the developer panel is showing.
    pub fn is_dev_panel_visible(&self) -> bool {
        self.dev_visible
    }

    /// Whether the settings overlay is showing.
    pub fn is_settings_open(&self) -> bool {
        self.settings_open
    }

    fn sync_page(&mut self, cx: &mut Context<Self>) {
        let page = page_for(self.store.read(cx).store());
        if page != self.page {
            self.page = page;
            let show = page == Page::Workspace;
            self.status.update(cx, |status, cx| status.set_show_commands(show, cx));
            cx.notify();
        }
    }

    fn toggle_dev_panel(&mut self, _: &ToggleDevPanel, _: &mut Window, cx: &mut Context<Self>) {
        if self.dev.is_some() {
            self.dev_visible = !self.dev_visible;
            cx.notify();
        }
    }

    fn go_to_needs_me(&mut self, _: &GoToNeedsMe, window: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Workspace {
            self.workspace.update(cx, |workspace, cx| workspace.go_to_needs_me(window, cx));
        }
    }

    fn toggle_pin(&mut self, _: &TogglePinContext, window: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Workspace {
            Workspace::toggle_pin(&self.workspace, window, cx);
        }
    }

    fn open_settings(&mut self, _: &OpenSettings, _: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = true;
        cx.notify();
    }

    fn close_settings(&mut self, _: &CloseSettings, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_open {
            self.settings_open = false;
            window.focus(&self.focus_handle, cx);
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    fn toggle_theme(&mut self, _: &ToggleTheme, window: &mut Window, cx: &mut Context<Self>) {
        let mode = if cx.theme().is_dark() { ThemeMode::Light } else { ThemeMode::Dark };
        Theme::change(mode, Some(window), cx);
        cx.refresh_windows();
    }
}

fn unlock_handler(store: &Entity<StoreHandle>) -> UnlockHandler {
    let store = store.downgrade();
    Rc::new(move |account_id: AccountId, secret: SecretInput, _: &mut Window, cx: &mut App| {
        let intent = Intent::human(IntentKind::Unlock { account_id, secret });
        store.update(cx, |store, cx| store.dispatch(intent, cx)).ok();
    })
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.page {
            Page::Entry(_) => self.entry.clone().into_any_element(),
            Page::Workspace => self.workspace.clone().into_any_element(),
        };
        v_flex()
            .id("shell")
            .key_context("Shell")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::toggle_dev_panel))
            .on_action(cx.listener(Self::go_to_needs_me))
            .on_action(cx.listener(Self::toggle_pin))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::close_settings))
            .on_action(cx.listener(Self::toggle_theme))
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.status.clone())
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(content)
                            .when(self.settings_open, |area| area.child(SettingsPage)),
                    )
                    .when_some(self.dev.clone().filter(|_| self.dev_visible), |row, dev| {
                        row.child(dev)
                    }),
            )
    }
}
