// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The settings page (P2): a full-window overlay with one section per area.
//!
//! Every control maps to an intent (emitted as a UI event through the root
//! sink, scope `Panel(settings.<section>)`, or, for passphrases, handed to
//! the root as a secret) or is marked view-only: the MCP and A2I lists are
//! read-only by design, and appearance is a local preference.
//!
//! - [`account`]: lock, passphrase rotation, backup export (with its own
//!   passphrase).
//! - [`backup`]: full backup and full restore, with progress, the review
//!   of a checked backup, and the final counts.
//! - [`agents`]: AI participants the open account custodies: creating one,
//!   listing them, and exporting their one-shot credentials.
//! - [`devices`]: devices, revocation (confirmed), own-device sync.
//! - [`nodes`]: node profiles, probe results, default, enable, remove.
//! - [`integrations`]: MCP servers and pending A2I events (read-only).
//! - [`diagnostics`]: redacted export preview and export.
//! - [`sync`]: how the account is kept up to date and the polling fallback
//!   (local preference).
//! - [`local_setup`]: source, socket, daemon program and data folder, the
//!   local daemon, version and log locations (local preference).
//! - [`appearance`]: theme and language (local preference).

mod account;
pub mod agents;
pub mod appearance;
pub mod backup;
pub(crate) mod design;
pub mod devices;
pub mod diagnostics;
mod errors;
pub mod integrations;
pub mod local_setup;
pub mod nodes;
pub(crate) mod presentation;
pub mod sync;

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::FocusTrapElement as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::setting::Settings;
use gpui_kit::component::{
    ActiveTheme as _, IconName, Selectable as _, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, Render, SharedString, Subscription,
    Window, div,
};
use ramflux_desktop_core::Scope;

use crate::actions::{CloseSettings, SETTINGS_CONTEXT};
use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::tr;
use crate::launch::DaemonLauncher;
use crate::root::{ConfirmHost, IntentRoot, RootEvent};

/// A settings section, in sidebar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SettingsSection {
    /// Account.
    Account,
    /// Full backup and restore.
    Backup,
    /// AI participants the open account custodies.
    Agents,
    /// Devices.
    Devices,
    /// Nodes.
    Nodes,
    /// MCP and A2I.
    Integrations,
    /// Diagnostics.
    Diagnostics,
    /// Appearance.
    Appearance,
}

impl SettingsSection {
    /// Every section in sidebar order.
    pub const ALL: [Self; 8] = [
        Self::Account,
        Self::Backup,
        Self::Agents,
        Self::Devices,
        Self::Nodes,
        Self::Integrations,
        Self::Diagnostics,
        Self::Appearance,
    ];

    /// Stable key, used for scopes and element ids.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::Backup => "backup",
            Self::Agents => "agents",
            Self::Devices => "devices",
            Self::Nodes => "nodes",
            Self::Integrations => "integrations",
            Self::Diagnostics => "diagnostics",
            Self::Appearance => "appearance",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
}

/// What every section renderer needs.
#[derive(Clone)]
pub struct SectionContext {
    /// The store handle (read-only).
    pub store: Entity<StoreHandle>,
    /// The root sink scoped to `Panel(settings)`.
    pub sink: EventSink,
    /// The intent root (for secret-carrying intents).
    pub root: Entity<IntentRoot>,
    /// The local daemon launcher and launch settings.
    pub launcher: Entity<DaemonLauncher>,
}

impl SectionContext {
    /// The sink scoped to one section.
    pub fn sink(&self, section: SettingsSection) -> EventSink {
        self.sink.scoped(Scope::Panel(format!("settings.{}", section.key())))
    }
}

/// The settings overlay.
pub struct SettingsPage {
    context: SectionContext,
    inputs: account::AccountInputs,
    backup: Entity<backup::BackupPanel>,
    agents: Entity<agents::AgentsPanel>,
    launch_inputs: local_setup::LaunchInputs,
    section: SettingsSection,
    opened: usize,
    errors: errors::ErrorScope,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for SettingsPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl SettingsPage {
    /// A settings page over `store`, emitting through `root`.
    pub fn new(
        store: Entity<StoreHandle>,
        root: Entity<IntentRoot>,
        launcher: Entity<DaemonLauncher>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut subscriptions = vec![
            cx.subscribe(&store, |this, store, event: &StoreEvent, cx| match event {
                StoreEvent::Changed(changes) => {
                    let handle = store.read(cx);
                    let account = handle.active_account().cloned();
                    this.errors.observe(account.as_ref(), changes, handle.store());
                    if changes.touches_settings() || changes.touches_docks() {
                        cx.notify();
                    }
                }
                StoreEvent::BackgroundChanged(account) => {
                    if let Some(own) = store.read(cx).account_store(account) {
                        this.errors.note_listed(account, own);
                    }
                }
                StoreEvent::Inspected | StoreEvent::CallAudio => {}
            }),
            cx.observe(&root, |_, _, cx| cx.notify()),
            cx.subscribe_in(&root, window, |this, root, event: &RootEvent, window, cx| {
                if *event == RootEvent::ConfirmClosed
                    && root.read(cx).host() == ConfirmHost::Settings
                {
                    window.focus(&this.focus_handle, cx);
                }
            }),
            cx.observe(&launcher, |_, _, cx| cx.notify()),
        ];
        let launch_inputs = local_setup::LaunchInputs::new(&launcher, window, cx);
        let masked = |key: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            crate::i18n::localized_input(key, window, cx, |input| input.masked(true))
        };
        let inputs = account::AccountInputs {
            current: masked("settings.account.current", window, cx),
            next: masked("settings.account.next", window, cx),
            backup: masked("settings.account.backup_passphrase", window, cx),
        };
        // The export control enables once a backup passphrase is typed.
        subscriptions.push(cx.subscribe(&inputs.backup, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        }));
        let backup = cx.new(|cx| backup::BackupPanel::new(store.clone(), root.clone(), window, cx));
        subscriptions.push(cx.observe(&backup, |_, _, cx| cx.notify()));
        let agents = cx.new(|cx| agents::AgentsPanel::new(store.clone(), root.clone(), window, cx));
        subscriptions.push(cx.observe(&agents, |_, _, cx| cx.notify()));
        let sink = IntentRoot::sink(&root).scoped(Scope::Panel("settings".into()));
        Self {
            context: SectionContext { store, sink, root, launcher },
            inputs,
            backup,
            agents,
            launch_inputs,
            section: SettingsSection::Account,
            opened: 0,
            errors: errors::ErrorScope::default(),
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Shows `section` the next time the overlay renders.
    pub fn show(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        if self.section != section {
            self.context.root.update(cx, |root, cx| root.cancel_for_navigation(cx));
        }
        self.section = section;
        self.opened += 1;
        // Showing a section starts its error banner afresh for the rendered
        // account; a directory still waiting for its first answer keeps the
        // failure that explains why it is not loading.
        let handle = self.context.store.read(cx);
        let account = handle.active_account().cloned();
        let waiting =
            section == SettingsSection::Devices && !self.errors.devices_listed(account.as_ref());
        if !waiting {
            self.errors.clear(account.as_ref(), section, handle.store());
        }
        cx.notify();
    }

    /// The section shown when the overlay opened.
    pub fn section(&self) -> SettingsSection {
        self.section
    }

    /// Shows the section `step` places after (or, negative, before) the
    /// current one, wrapping around (keyboard section navigation).
    pub fn step_section(&mut self, step: isize, cx: &mut Context<Self>) {
        let count = SettingsSection::ALL.len();
        let current = isize::try_from(self.section.index()).unwrap_or(0);
        let len = isize::try_from(count).unwrap_or(1);
        let next = usize::try_from((current + step.rem_euclid(len)).rem_euclid(len)).unwrap_or(0);
        if let Some(section) = SettingsSection::ALL.get(next) {
            self.show(*section, cx);
        }
    }

    /// The masked inputs of passphrase rotation.
    pub fn rotate_inputs(&self) -> [&Entity<InputState>; 2] {
        [&self.inputs.current, &self.inputs.next]
    }

    /// The backup section's panel.
    pub fn backup_panel(&self) -> &Entity<backup::BackupPanel> {
        &self.backup
    }

    /// The AI participants section's panel.
    pub fn agents_panel(&self) -> &Entity<agents::AgentsPanel> {
        &self.agents
    }

    /// The masked backup passphrase input.
    pub fn backup_input(&self) -> &Entity<InputState> {
        &self.inputs.backup
    }
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let context = &self.context;
        let page = match self.section {
            SettingsSection::Account => account::page(context, &self.inputs, cx),
            SettingsSection::Backup => backup::page(&self.backup, cx),
            SettingsSection::Agents => agents::page(&self.agents),
            SettingsSection::Devices => devices::page(context, &self.errors, cx),
            SettingsSection::Nodes => nodes::page(context, cx),
            SettingsSection::Integrations => integrations::page(context, cx),
            SettingsSection::Diagnostics => diagnostics::page(context, &self.launch_inputs, cx),
            SettingsSection::Appearance => appearance::page(cx),
        };
        let root = context.root.read(cx);
        let strip = (root.host() == ConfirmHost::Settings).then(|| root.confirm_strip().clone());
        let palette = ramflux_desktop_navigation::palette(cx);
        let close = Button::new("settings-close")
            .ghost()
            .small()
            .icon(IconName::Close)
            .accessibility_label(tr("common.close"))
            .tooltip(tr("common.close"))
            .on_click(|_, window, cx| window.dispatch_action(Box::new(CloseSettings), cx));
        let compact =
            f32::from(window.viewport_size().width) / f32::from(window.rem_size()) < 800.0 / 15.0;
        // The page column is at most 760 px wide and centred in the content
        // area (the window less the rail and the 264 px section list); the
        // Kit page keeps its own 1 rem inset inside it.
        let column_pad = {
            let rem = f32::from(window.rem_size());
            let content = f32::from(window.viewport_size().width)
                - if compact {
                    0.0
                } else {
                    rem * (ramflux_desktop_navigation::theme::RAIL_REM
                        + ramflux_desktop_navigation::theme::SECTION_REM)
                };
            ((content - 760.0) / 2.0 - rem).max(32.0 - rem)
        };

        let offline = context.store.read(cx).store().source_status()
            == ramflux_desktop_core::SourceStatus::Disconnected;
        let failure = {
            let handle = context.store.read(cx);
            self.errors
                .current(handle.active_account(), self.section, handle.store())
                .map(|failure| crate::i18n::backend_text(&failure.message))
        };
        // Design G01–G13: the section list sits in the 264 px list pane under
        // a "Settings" title; the current page fills the content area.
        let navigation = div()
            .id("settings-navigation")
            .test_support()
            .flex()
            .gap_1()
            .when(compact, |nav| nav.flex_row().flex_wrap().p_2().border_b_1())
            .when(!compact, |nav| {
                nav.flex_col()
                    .w(gpui_kit::rems(ramflux_desktop_navigation::theme::SECTION_REM))
                    .px_3()
                    .pt(gpui_kit::rems(24.0 / 15.0))
                    .border_r_1()
                    .child(
                        div()
                            .id("settings-title")
                            .flex()
                            .px_2()
                            .pb_3()
                            .text_size(gpui_kit::rems(palette.size_page / 15.0))
                            .font_semibold()
                            .child(tr("settings.title")),
                    )
            })
            .flex_shrink_0()
            .border_color(palette.line)
            .bg(palette.nav)
            // Agents live in their own section of the rail (design 04 / D01);
            // their management page opens from there, not from this list.
            .children(
                SettingsSection::ALL.into_iter().filter(|s| *s != SettingsSection::Agents).map(
                    |section| {
                        Button::new(SharedString::from(format!(
                            "settings-section-{}",
                            section.key()
                        )))
                        .ghost()
                        .when(!compact, |button| {
                            button
                                .w_full()
                                .justify_start()
                                .px_3()
                                .h(gpui_kit::rems(40.0 / 15.0))
                                .rounded(gpui_kit::px(palette.radius_sm))
                                .text_size(gpui_kit::rems(palette.size_body / 15.0))
                        })
                        .when(compact, |button| button.small())
                        .selected(self.section == section)
                        // The current section is exposed to assistive technology,
                        // not only drawn as selected.
                        .toggled(self.section == section)
                        .accessibility_label(tr(&format!("settings.section.{}", section.key())))
                        .child(
                            div()
                                .flex_1()
                                .when(compact, |label| label.flex_none())
                                .child(tr(&format!("settings.section.{}", section.key()))),
                        )
                        // Focus stays on the pressed button, so keyboard users keep
                        // their ring and Tab position.
                        .on_click(cx.listener(move |this, _, _, cx| this.show(section, cx)))
                    },
                ),
            );
        v_flex()
            .id("settings")
            .test_support()
            .key_context(SETTINGS_CONTEXT)
            .absolute()
            .occlude()
            .inset_0()
            .min_w_0()
            .bg(cx.theme().background)
            .when(offline, |page| page.child(presentation::offline(cx)))
            .when_some(failure, |page, reason| {
                page.child(presentation::notice(
                    "settings-last-error",
                    tr("settings_ui.last_error"),
                    reason,
                    ramflux_desktop_navigation::generated::catalog::StateKind::Error,
                    cx,
                ))
            })
            .child(
                div()
                    .id("settings-content")
                    .test_support()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .when(compact, |content| content.flex_col())
                    .child(navigation)
                    .child(
                        // Design G01–G13: the page title in the page size with
                        // the close icon, a full-width rule, then the page in a
                        // centred 760 px column.
                        v_flex()
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .child(
                                h_flex()
                                    .id("settings-header")
                                    .test_support()
                                    .w_full()
                                    // The generated page header (88 px, 32 px
                                    // inset, rule) with the board's 24 px title.
                                    .child(
                                        ramflux_desktop_navigation::generated::catalog::page_header_content(
                                            ramflux_desktop_navigation::generated::catalog::RowContent::new(
                                                "settings-page".into(),
                                                page_title(self.section),
                                                SharedString::default(),
                                            )
                                            .title_size(palette.size_page)
                                            .title_weight(gpui_kit::FontWeight::SEMIBOLD)
                                            .trailing(close),
                                            false,
                                            palette,
                                        ),
                                    ),
                            )
                            .child(
                                div()
                                    .relative()
                                    .flex_1()
                                    .min_h_0()
                                    .px(gpui_kit::px(column_pad))
                                    .child(
                                        Settings::new(SharedString::from(format!(
                                            "settings-pages-{}",
                                            self.opened
                                        )))
                                        // Navigation is owned here so pointer and keyboard
                                        // changes cancel a host confirmation before hiding facts.
                                        .sidebar_width(gpui_kit::px(0.0))
                                        .sidebar_size_range(gpui_kit::px(0.0)..gpui_kit::px(0.0))
                                        .sidebar_style(
                                            &gpui_kit::StyleRefinement::default().hidden(),
                                        )
                                        .page(
                                            page.header_style(
                                                &gpui_kit::StyleRefinement::default().hidden(),
                                            ),
                                        ),
                                    )
                                    // Kit's Settings keeps the divider of its
                                    // (hidden, zero-width) sidebar; the design
                                    // column has no rule, so it is covered.
                                    .child(
                                        div()
                                            .absolute()
                                            .top_0()
                                            .bottom_0()
                                            .left(gpui_kit::px(column_pad - 2.0))
                                            .w(gpui_kit::px(6.0))
                                            .bg(palette.bg)
                                            .occlude(),
                                    ),
                            ),
                    ),
            )
            .children(strip.map(|strip| div().flex_shrink_0().child(strip)))
            // An overlay over the whole window: Tab stays inside it. The trap
            // tracks the page's focus handle and keeps the "settings" id.
            .focus_trap("settings", &self.focus_handle)
    }
}

/// The page title of a section: its list label, except where the design
/// names the page more fully (Appearance → "Appearance and language").
fn page_title(section: SettingsSection) -> SharedString {
    match section {
        SettingsSection::Appearance => tr("settings.appearance.title"),
        other => tr(&format!("settings.section.{}", other.key())),
    }
}

/// A muted note line used by sections (for example "view-only").
pub(crate) fn note(key: &str, cx: &App) -> gpui_kit::AnyElement {
    note_text(tr(key), cx)
}

/// A muted note line of already translated text.
pub(crate) fn note_text(text: SharedString, cx: &App) -> gpui_kit::AnyElement {
    div().text_xs().text_color(cx.theme().muted_foreground).child(text).into_any_element()
}
