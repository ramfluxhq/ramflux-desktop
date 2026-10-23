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
//! - [`devices`]: devices, revocation (hold), own-device sync.
//! - [`nodes`]: node profiles, probe results, default, enable, remove.
//! - [`integrations`]: MCP servers and pending A2I events (read-only).
//! - [`diagnostics`]: redacted export preview and export.
//! - [`appearance`]: theme and language (local preference).

pub mod account;
pub mod appearance;
pub mod devices;
pub mod diagnostics;
pub mod integrations;
pub mod nodes;

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::FocusTrapElement as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::setting::{SelectIndex, Settings};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
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
use crate::root::{ConfirmHost, IntentRoot};

/// A settings section, in sidebar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    /// Account.
    Account,
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
    pub const ALL: [Self; 6] = [
        Self::Account,
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
    section: SettingsSection,
    opened: usize,
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut subscriptions = vec![
            cx.subscribe(&store, |_, _, event: &StoreEvent, cx| {
                if let StoreEvent::Changed(changes) = event
                    && (changes.touches_settings() || changes.touches_docks())
                {
                    cx.notify();
                }
            }),
            cx.observe(&root, |_, _, cx| cx.notify()),
        ];
        let masked = |key: &str, window: &mut Window, cx: &mut Context<Self>| {
            let placeholder = tr(key);
            cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(placeholder))
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
        let sink = IntentRoot::sink(&root).scoped(Scope::Panel("settings".into()));
        Self {
            context: SectionContext { store, sink, root },
            inputs,
            section: SettingsSection::Account,
            opened: 0,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Shows `section` the next time the overlay renders.
    pub fn show(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        self.section = section;
        self.opened += 1;
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
        let next = usize::try_from((current + step).rem_euclid(len)).unwrap_or(0);
        self.show(SettingsSection::ALL[next], cx);
    }

    /// The masked inputs of passphrase rotation.
    pub fn rotate_inputs(&self) -> [&Entity<InputState>; 2] {
        [&self.inputs.current, &self.inputs.next]
    }

    /// The masked backup passphrase input.
    pub fn backup_input(&self) -> &Entity<InputState> {
        &self.inputs.backup
    }
}

impl Render for SettingsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let context = &self.context;
        let pages = [
            account::page(context, &self.inputs, cx),
            devices::page(context, cx),
            nodes::page(context, cx),
            integrations::page(context, cx),
            diagnostics::page(context, cx),
            appearance::page(),
        ];
        let root = context.root.read(cx);
        let strip = (root.host() == ConfirmHost::Settings).then(|| root.confirm_strip().clone());
        v_flex()
            .id("settings")
            .test_support()
            .key_context(SETTINGS_CONTEXT)
            .absolute()
            .inset_0()
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .px_4()
                    .py_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().flex_1().font_semibold().child(tr("settings.title")))
                    .child(
                        Button::new("settings-close")
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .label(tr("common.close"))
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(CloseSettings), cx)
                            }),
                    ),
            )
            .child(
                div().flex_1().min_h_0().child(
                    Settings::new(SharedString::from(format!("settings-pages-{}", self.opened)))
                        .default_selected_index(SelectIndex {
                            page_ix: self.section.index(),
                            group_ix: None,
                        })
                        .pages(pages),
                ),
            )
            .children(strip.map(|strip| div().flex_shrink_0().pb_2().child(strip)))
            // An overlay over the whole window: Tab stays inside it. The trap
            // tracks the page's focus handle and keeps the "settings" id.
            .focus_trap("settings", &self.focus_handle)
    }
}

/// A muted note line used by sections (for example "view-only").
pub(crate) fn note(key: &str, cx: &App) -> gpui_kit::AnyElement {
    div().text_xs().text_color(cx.theme().muted_foreground).child(tr(key)).into_any_element()
}
