// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The window root: title bar status strip, the page the store calls for
//! (entry or workspace), the settings overlay, the command palette and the
//! developer panel.
//!
//! Window-wide commands are handled here and routed to the view that owns
//! them, so a command works the same from a key binding, the title bar, the
//! palette or a menu. The shell also tells the intent root which page hosts
//! the hold-to-confirm strip.

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, Render, Subscription, Window, div,
};
use ramflux_desktop_core::{ContextKind, selectors};
use ramflux_desktop_sdk::{SdkConfig, SdkHub};
use ramflux_desktop_test::load_fixture;

use crate::actions::{
    ClosePalette, CloseSettings, GoToNeedsMe, NextSettingsSection, OpenDiagnosticsSettings,
    OpenPalette, OpenSettings, PreviousSettingsSection, SelectStackEntry1, SelectStackEntry2,
    SelectStackEntry3, SelectStackEntry4, SelectStackEntry5, SelectStackEntry6, StartLocalDaemon,
    StopLocalDaemon, ToggleApprovalsDock, ToggleContactsDock, ToggleDevPanel, ToggleEvidenceDock,
    ToggleMembersDock, ToggleObjectsDock, TogglePinContext, ToggleTheme,
};
use crate::app::{StoreEvent, StoreHandle};
use crate::dev::DevPanel;
use crate::entry::EntryPage;
use crate::launch::{DaemonLauncher, LaunchOptions};
use crate::palette::{CommandPalette, PaletteChoice, PaletteEvent};
use crate::root::{ConfirmHost, IntentRoot};
use crate::route::{EntryStep, Page, page_for};
use crate::settings::{SettingsPage, SettingsSection};
use crate::workspace::{DockTool, PrefsStore, StatusStrip, Workspace};

/// Scenario played when the application starts.
pub const DEFAULT_SCENARIO: &str = "s01_first_launch";

/// File name of the desktop settings (pinned contexts, launch settings) in
/// the state directory.
pub const DESKTOP_SETTINGS_FILE: &str = ramflux_desktop_sdk::local::paths::SETTINGS_FILE;

/// Where the data comes from.
#[derive(Debug, Clone)]
pub enum SourceChoice {
    /// The bundled scenario named in [`ShellOptions::scenario`].
    Mock,
    /// A real daemon over its local bus.
    Rfd(SdkConfig),
}

impl SourceChoice {
    /// The default: scenarios in debug builds, the daemon at its default
    /// socket in release builds.
    pub fn default_for_build() -> Self {
        if cfg!(debug_assertions) { Self::Mock } else { Self::Rfd(SdkConfig::default()) }
    }
}

/// How a shell is set up.
#[derive(Debug, Clone)]
pub struct ShellOptions {
    /// Where the data comes from.
    pub source: SourceChoice,
    /// Bundled scenario to play.
    pub scenario: String,
    /// Run the real-time clock (false: time moves only through the developer controls).
    pub start_clock: bool,
    /// Anchor playback to the wall clock so moment times are real local times.
    pub wall_clock: bool,
    /// Where the dock layout persists.
    pub prefs: PrefsStore,
    /// Launch settings, bundled daemon and daemon log of the launcher.
    pub launch: LaunchOptions,
}

impl ShellOptions {
    /// Deterministic options for tests and reference screenshots: fixture
    /// epoch, layout kept in memory.
    pub fn for_scenario(name: &str, start_clock: bool) -> Self {
        Self {
            source: SourceChoice::Mock,
            scenario: name.to_owned(),
            start_clock,
            wall_clock: false,
            prefs: PrefsStore::in_memory(),
            launch: LaunchOptions::in_memory(),
        }
    }
}

/// The window root view.
pub struct Shell {
    store: Entity<StoreHandle>,
    root: Entity<IntentRoot>,
    status: Entity<StatusStrip>,
    entry: Entity<EntryPage>,
    workspace: Entity<Workspace>,
    settings: Entity<SettingsPage>,
    palette: Entity<CommandPalette>,
    launcher: Entity<DaemonLauncher>,
    dev: Option<Entity<DevPanel>>,
    dev_visible: bool,
    settings_open: bool,
    palette_open: bool,
    page: Page,
    focus_handle: FocusHandle,
    /// Where keyboard focus returns when an overlay (settings, palette) closes.
    return_focus: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for Shell {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Shell {
    /// The application window root: [`DEFAULT_SCENARIO`] in real time,
    /// anchored to the wall clock, with the dock layout persisted.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::application(SourceChoice::default_for_build(), window, cx)
    }

    /// The application window root on `source`: scenarios in real time
    /// anchored to the wall clock, or the live daemon; the dock layout and
    /// (live) the pinned contexts are persisted.
    pub fn application(source: SourceChoice, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Pinned contexts persist in the desktop settings file beside the
        // dock layout.
        let source = match source {
            SourceChoice::Rfd(mut config) => {
                if config.settings_path.is_none() {
                    config.settings_path = ramflux_desktop_sdk::local::paths::settings_file();
                }
                SourceChoice::Rfd(config)
            }
            SourceChoice::Mock => SourceChoice::Mock,
        };
        let options = ShellOptions {
            source,
            scenario: DEFAULT_SCENARIO.to_owned(),
            start_clock: true,
            wall_clock: true,
            prefs: PrefsStore::default_location(),
            launch: LaunchOptions::application(),
        };
        Self::with_options(options, window, cx)
    }

    /// A root playing the bundled scenario `name` deterministically. With
    /// `start_clock` false time only moves through the developer controls.
    pub fn with_scenario(
        name: &str,
        start_clock: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_options(ShellOptions::for_scenario(name, start_clock), window, cx)
    }

    /// A root set up by `options`.
    pub fn with_options(
        options: ShellOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let store = cx.new(|cx| {
            let mut store = StoreHandle::new();
            store.set_wall_clock_anchor(options.wall_clock);
            match &options.source {
                SourceChoice::Mock => {
                    if let Ok(scenario) = load_fixture(&options.scenario) {
                        store.load_scenario(&scenario, cx);
                    }
                }
                SourceChoice::Rfd(config) => match SdkHub::start(config.clone()) {
                    Ok(hub) => store.attach_live(hub, cx),
                    // Without a bridge the entry page keeps waiting for the daemon.
                    Err(error) => ramflux_desktop_sdk::local::app_log::warn(&format!(
                        "cannot start the daemon bridge: {error}"
                    )),
                },
            }
            if options.start_clock {
                store.start_clock(cx);
            }
            store
        });
        let root = IntentRoot::attach(store.clone(), window, cx);
        let launcher = cx.new(|cx| DaemonLauncher::new(store.clone(), options.launch.clone(), cx));
        let status = cx.new(|cx| StatusStrip::new(store.clone(), cx));
        let entry =
            cx.new(|cx| EntryPage::new(store.clone(), root.clone(), launcher.clone(), window, cx));
        let workspace = cx.new(|cx| {
            Workspace::new(store.clone(), root.clone(), options.prefs.clone(), window, cx)
        });
        let settings = cx
            .new(|cx| SettingsPage::new(store.clone(), root.clone(), launcher.clone(), window, cx));
        let palette =
            cx.new(|cx| CommandPalette::new(store.clone(), IntentRoot::sink(&root), window, cx));
        let dev = cfg!(debug_assertions)
            .then(|| cx.new(|cx| DevPanel::new(store.clone(), workspace.downgrade(), cx)));
        let subscriptions = vec![
            cx.subscribe_in(&store, window, |this, _, event: &StoreEvent, _, cx| {
                if let StoreEvent::Changed(changes) = event
                    && changes.touches_status()
                {
                    this.sync_page(cx);
                }
            }),
            cx.subscribe_in(&palette, window, |this, _, event: &PaletteEvent, window, cx| {
                this.on_palette_event(event.clone(), window, cx);
            }),
            // A focused control can stop rendering (a confirmed node card, a
            // closed strip, a finished entry step). Keyboard focus then moves
            // to its closest rendered ancestor, or the window root, so Tab
            // and every shortcut keep working.
            cx.on_focus_lost(window, |this: &mut Self, window, cx| {
                let target = window
                    .focus_lost_restore_target(cx)
                    .unwrap_or_else(|| this.focus_handle.clone());
                target.focus(window, cx);
            }),
        ];
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let mut shell = Self {
            store,
            root,
            status,
            entry,
            workspace,
            settings,
            palette,
            launcher,
            dev,
            dev_visible: false,
            settings_open: false,
            palette_open: false,
            page: Page::Entry(EntryStep::Daemon),
            focus_handle,
            return_focus: None,
            _subscriptions: subscriptions,
        };
        shell.sync_page(cx);
        shell
    }

    /// The store handle.
    pub fn store(&self) -> &Entity<StoreHandle> {
        &self.store
    }

    /// The intent root.
    pub fn root(&self) -> &Entity<IntentRoot> {
        &self.root
    }

    /// The workspace page.
    pub fn workspace(&self) -> &Entity<Workspace> {
        &self.workspace
    }

    /// The entry page.
    pub fn entry(&self) -> &Entity<EntryPage> {
        &self.entry
    }

    /// The settings overlay.
    pub fn settings(&self) -> &Entity<SettingsPage> {
        &self.settings
    }

    /// The command palette.
    pub fn palette(&self) -> &Entity<CommandPalette> {
        &self.palette
    }

    /// The local daemon launcher.
    pub fn launcher(&self) -> &Entity<DaemonLauncher> {
        &self.launcher
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

    /// Whether the command palette is showing.
    pub fn is_palette_open(&self) -> bool {
        self.palette_open
    }

    fn sync_page(&mut self, cx: &mut Context<Self>) {
        let page = page_for(self.store.read(cx).store());
        if page != self.page {
            self.page = page;
            let show = page == Page::Workspace;
            self.status.update(cx, |status, cx| status.set_show_commands(show, cx));
            if !show {
                self.settings_open = false;
                self.palette_open = false;
            }
            cx.notify();
        }
        self.sync_host(cx);
    }

    fn sync_host(&mut self, cx: &mut Context<Self>) {
        let host = if self.settings_open {
            ConfirmHost::Settings
        } else if self.page == Page::Workspace {
            ConfirmHost::Workspace
        } else {
            ConfirmHost::Entry
        };
        self.root.update(cx, |root, cx| root.set_host(host, cx));
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

    /// Opens the settings overlay at `section`.
    pub fn open_settings_at(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        self.palette_open = false;
        self.settings_open = true;
        self.settings.update(cx, |page, cx| page.show(section, cx));
        self.sync_host(cx);
        cx.notify();
    }

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        if !self.settings_open {
            self.remember_focus(window, cx);
        }
        self.open_settings_at(SettingsSection::Account, cx);
        let focus = self.settings.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }

    fn open_diagnostics(
        &mut self,
        _: &OpenDiagnosticsSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.settings_open {
            self.remember_focus(window, cx);
        }
        self.open_settings_at(SettingsSection::Diagnostics, cx);
        let focus = self.settings.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }

    fn start_daemon(&mut self, _: &StartLocalDaemon, _: &mut Window, cx: &mut Context<Self>) {
        self.launcher.update(cx, |launcher, cx| launcher.start(cx));
    }

    fn stop_daemon(&mut self, _: &StopLocalDaemon, _: &mut Window, cx: &mut Context<Self>) {
        self.launcher.update(cx, |launcher, cx| launcher.stop(cx));
    }

    /// Saves the focused element so closing an overlay can return to it.
    fn remember_focus(&mut self, window: &mut Window, cx: &App) {
        self.return_focus = window.focused(cx);
    }

    /// Returns keyboard focus after an overlay closed: to the element that
    /// had it, or the window root when that element is gone.
    fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.return_focus.take().unwrap_or_else(|| self.focus_handle.clone());
        window.focus(&target, cx);
    }

    fn select_stack_entry(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Workspace {
            self.workspace.update(cx, |w, cx| w.select_stack_entry(ix, window, cx));
        }
    }

    fn toggle_dock(&mut self, tool: DockTool, window: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Workspace {
            self.workspace.update(cx, |w, cx| w.toggle_tool_with_focus(tool, window, cx));
        }
    }

    fn settings_section(&mut self, step: isize, cx: &mut Context<Self>) {
        if self.settings_open {
            self.settings.update(cx, |page, cx| page.step_section(step, cx));
        } else {
            cx.propagate();
        }
    }

    /// Closes the settings overlay.
    pub fn close_settings_now(&mut self, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.sync_host(cx);
        cx.notify();
    }

    fn close_settings(&mut self, _: &CloseSettings, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_open {
            self.settings_open = false;
            self.sync_host(cx);
            self.restore_focus(window, cx);
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    fn open_palette(&mut self, _: &OpenPalette, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Workspace {
            return;
        }
        let current = self.workspace.read(cx).current_context().cloned();
        if !self.palette_open {
            self.remember_focus(window, cx);
        }
        self.settings_open = false;
        self.sync_host(cx);
        self.palette_open = true;
        self.palette.update(cx, |palette, cx| palette.open(current, window, cx));
        cx.notify();
    }

    fn close_palette(&mut self, _: &ClosePalette, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_palette(window, cx);
    }

    fn dismiss_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette_open {
            self.palette_open = false;
            self.restore_focus(window, cx);
            cx.notify();
        }
    }

    fn on_palette_event(
        &mut self,
        event: PaletteEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_palette(window, cx);
        let PaletteEvent::Navigate(choice) = event else { return };
        let workspace = self.workspace.clone();
        match choice {
            PaletteChoice::Context(id) => {
                workspace.update(cx, |w, cx| w.select_context(Some(id), window, cx));
            }
            PaletteChoice::Moment(context, moment) => workspace.update(cx, |w, cx| {
                w.select_context(Some(context), window, cx);
                w.conversation().update(cx, |field, cx| field.focus_moment(&moment, cx));
            }),
            PaletteChoice::Object(_) => {
                workspace.update(cx, |w, cx| w.open_tool(DockTool::Objects, window, cx))
            }
            PaletteChoice::Contact(participant) => {
                let direct = {
                    let store = self.store.read(cx).store();
                    selectors::contexts_with(store, &participant)
                        .into_iter()
                        .find(|c| c.kind == ContextKind::Direct)
                        .map(|c| c.id.clone())
                };
                workspace.update(cx, |w, cx| match direct {
                    Some(id) => w.select_context(Some(id), window, cx),
                    None => w.open_tool(DockTool::Contacts, window, cx),
                });
            }
            PaletteChoice::SwitchAccount(account) => {
                self.store.update(cx, |store, cx| store.set_active_account(&account, cx));
            }
            PaletteChoice::OpenSettings => self.open_settings_at(SettingsSection::Account, cx),
            PaletteChoice::OpenDock(tool) => {
                workspace.update(cx, |w, cx| w.open_tool(tool, window, cx))
            }
            PaletteChoice::AddContact => {
                workspace.update(cx, |w, cx| w.open_add_contact(window, cx));
            }
            PaletteChoice::NeedsMe => workspace.update(cx, |w, cx| w.go_to_needs_me(window, cx)),
            PaletteChoice::ToggleTheme => self.toggle_theme(&ToggleTheme, window, cx),
            PaletteChoice::Pin(..) | PaletteChoice::Mute(..) | PaletteChoice::Disappearing(..) => {}
        }
    }

    fn toggle_theme(&mut self, _: &ToggleTheme, window: &mut Window, cx: &mut Context<Self>) {
        let mode = if cx.theme().is_dark() { ThemeMode::Light } else { ThemeMode::Dark };
        Theme::change(mode, Some(window), cx);
        cx.refresh_windows();
    }
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
            .on_action(cx.listener(Self::open_diagnostics))
            .on_action(cx.listener(Self::start_daemon))
            .on_action(cx.listener(Self::stop_daemon))
            .on_action(cx.listener(Self::close_settings))
            .on_action(cx.listener(Self::open_palette))
            .on_action(cx.listener(Self::close_palette))
            .on_action(cx.listener(Self::toggle_theme))
            .on_action(
                cx.listener(|this, _: &SelectStackEntry1, w, cx| this.select_stack_entry(0, w, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SelectStackEntry2, w, cx| this.select_stack_entry(1, w, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SelectStackEntry3, w, cx| this.select_stack_entry(2, w, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SelectStackEntry4, w, cx| this.select_stack_entry(3, w, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SelectStackEntry5, w, cx| this.select_stack_entry(4, w, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SelectStackEntry6, w, cx| this.select_stack_entry(5, w, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleObjectsDock, w, cx| {
                this.toggle_dock(DockTool::Objects, w, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleContactsDock, w, cx| {
                this.toggle_dock(DockTool::Contacts, w, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleMembersDock, w, cx| {
                this.toggle_dock(DockTool::Members, w, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleApprovalsDock, w, cx| {
                this.toggle_dock(DockTool::Approvals, w, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleEvidenceDock, w, cx| {
                this.toggle_dock(DockTool::Evidence, w, cx)
            }))
            .on_action(
                cx.listener(|this, _: &NextSettingsSection, _, cx| this.settings_section(1, cx)),
            )
            .on_action(
                cx.listener(|this, _: &PreviousSettingsSection, _, cx| {
                    this.settings_section(-1, cx)
                }),
            )
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
                            .when(self.settings_open, |area| area.child(self.settings.clone()))
                            .when(self.palette_open, |area| area.child(self.palette.clone())),
                    )
                    .when_some(self.dev.clone().filter(|_| self.dev_visible), |row, dev| {
                        row.child(dev)
                    }),
            )
    }
}
