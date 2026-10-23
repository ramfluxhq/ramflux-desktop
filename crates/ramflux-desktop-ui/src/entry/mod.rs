// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The entry page (P0): daemon, then node, then account.
//!
//! The step comes from the store ([`crate::route::page_for`]); the user may
//! look back at the node step from the account step (view-local). Every path
//! in is enabled: confirm a discovered node or add one by URL (NODE-01 card,
//! trust-on-first-use pins need a hold), unlock, create an account, join as
//! a new device, or restore a backup.
//!
//! Non-secret decisions bubble as UI events through the root sink (scope
//! `Panel(entry)`). Secrets are typed only here: they leave their masked
//! input as a [`ramflux_desktop_core::SecretInput`] straight into
//! [`IntentRoot::submit_secret`], never as a UI event, and the input is
//! cleared immediately.
//!
//! - `node_step`: node cards and the add-node form.
//! - `account_step`: unlock, create, join and restore forms.

mod account_step;
mod node_card;
mod node_step;

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::stepper::{Stepper, StepperItem};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, Entity, IntoElement, Render, Subscription, Window, div, rems};
use ramflux_desktop_core::{AccountId, AccountKind, NodeId, Scope};

use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::tr;
use crate::root::{ConfirmHost, IntentRoot};
use crate::route::{EntryStep, Page, page_for};

pub use node_card::NodeCard;

/// Which account path the account step shows (view-local).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountPath {
    /// Create a new identity.
    Create,
    /// Join an existing identity as a new device.
    Join,
    /// Restore from an encrypted backup.
    Restore,
}

/// The text inputs of the entry page.
struct Inputs {
    passphrase: Entity<InputState>,
    node_url: Entity<InputState>,
    label: Entity<InputState>,
    new_secret: Entity<InputState>,
    device_label: Entity<InputState>,
    code: Entity<InputState>,
    backup_path: Entity<InputState>,
    backup_secret: Entity<InputState>,
}

impl Inputs {
    fn new(window: &mut Window, cx: &mut App) -> Self {
        let plain = |key: &str, window: &mut Window, cx: &mut App| {
            let placeholder = tr(key);
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let masked = |key: &str, window: &mut Window, cx: &mut App| {
            let placeholder = tr(key);
            cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(placeholder))
        };
        Self {
            passphrase: masked("entry.account.passphrase", window, cx),
            node_url: plain("entry.node.url_placeholder", window, cx),
            label: plain("entry.create.label_placeholder", window, cx),
            new_secret: masked("entry.create.passphrase", window, cx),
            device_label: plain("entry.join.device_placeholder", window, cx),
            code: masked("entry.join.code", window, cx),
            backup_path: plain("entry.restore.path_placeholder", window, cx),
            backup_secret: masked("entry.restore.passphrase", window, cx),
        }
    }
}

/// The entry page view.
pub struct EntryPage {
    store: Entity<StoreHandle>,
    root: Entity<IntentRoot>,
    sink: EventSink,
    inputs: Inputs,
    path: AccountPath,
    kind: AccountKind,
    node: Option<NodeId>,
    review_nodes: bool,
    submitted: Option<AccountId>,
    _subscriptions: Vec<Subscription>,
}

impl EntryPage {
    /// An entry page reading `store` and emitting through `root`.
    pub fn new(
        store: Entity<StoreHandle>,
        root: Entity<IntentRoot>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let inputs = Inputs::new(window, cx);
        let subscriptions = vec![
            cx.subscribe(&store, |this, _, event: &StoreEvent, cx| {
                if let StoreEvent::Changed(changes) = event
                    && changes.touches_status()
                {
                    this.submitted = None;
                    cx.notify();
                }
            }),
            cx.observe(&root, |_, _, cx| cx.notify()),
            cx.subscribe_in(&inputs.passphrase, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.unlock(window, cx);
                }
            }),
            cx.subscribe_in(&inputs.node_url, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.add_node(window, cx);
                }
            }),
        ];
        Self {
            sink: IntentRoot::sink(&root).scoped(Scope::Panel("entry".into())),
            store,
            root,
            inputs,
            path: AccountPath::Create,
            kind: AccountKind::HumanInteractive,
            node: None,
            review_nodes: false,
            submitted: None,
            _subscriptions: subscriptions,
        }
    }

    /// The unlock passphrase input.
    pub fn passphrase(&self) -> &Entity<InputState> {
        &self.inputs.passphrase
    }

    /// Every masked secret input, for tests that check secrets are cleared.
    pub fn secret_inputs(&self) -> [&Entity<InputState>; 4] {
        [
            &self.inputs.passphrase,
            &self.inputs.new_secret,
            &self.inputs.code,
            &self.inputs.backup_secret,
        ]
    }

    /// The step shown: the store's step, or the node step while the user
    /// reviews nodes from the account step.
    pub fn step(&self, cx: &App) -> EntryStep {
        match page_for(self.store.read(cx).store()) {
            Page::Entry(EntryStep::Account) if self.review_nodes && !self.has_account(cx) => {
                EntryStep::Node
            }
            Page::Entry(step) => step,
            Page::Workspace => EntryStep::Account,
        }
    }

    fn has_account(&self, cx: &App) -> bool {
        self.store.read(cx).store().accounts().next().is_some()
    }

    fn render_daemon(&self, cx: &App) -> impl IntoElement {
        let socket = self
            .store
            .read(cx)
            .live_socket()
            .map(|path| crate::i18n::tr_args("backend.daemon.waiting", &[("path", path)]));
        h_flex().id("entry-daemon").gap_3().child(Spinner::new()).child(
            v_flex()
                .gap_1()
                .child(div().text_sm().child(tr("entry.daemon.waiting")))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("entry.daemon.help")),
                )
                .when_some(socket, |col, socket| {
                    col.child(
                        div()
                            .text_xs()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_color(cx.theme().muted_foreground)
                            .child(socket),
                    )
                }),
        )
    }
}

impl Render for EntryPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let step = self.step(cx);
        let selected = match step {
            EntryStep::Daemon => 0,
            EntryStep::Node => 1,
            EntryStep::Account => 2,
        };
        let content = match step {
            EntryStep::Daemon => self.render_daemon(cx).into_any_element(),
            EntryStep::Node => self.render_node_step(cx).into_any_element(),
            EntryStep::Account => self.render_account_step(window, cx).into_any_element(),
        };
        let root = self.root.read(cx);
        let strip = (root.host() == ConfirmHost::Entry).then(|| root.confirm_strip().clone());
        let scroll = div().id("entry").test_support().flex_1().min_h_0().overflow_y_scroll().child(
            v_flex()
                .mx_auto()
                .w_full()
                .max_w(rems(38.))
                .gap_6()
                .px_6()
                .py_12()
                .child(
                    v_flex()
                        .gap_1()
                        .child(div().text_xl().font_semibold().child(tr("entry.title")))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr("entry.subtitle")),
                        ),
                )
                .child(Stepper::new("entry-steps").selected_index(selected).items([
                    StepperItem::new().child(tr("entry.step.daemon")),
                    StepperItem::new().child(tr("entry.step.node")),
                    StepperItem::new().child(tr("entry.step.account")),
                ]))
                .child(content),
        );
        // The hold strip stays in view at the foot of the page. It is a
        // footer that reserves its own height (it never shrinks), separated
        // by a rule, so the scrolling content ends above it and is never
        // drawn underneath.
        v_flex().size_full().bg(cx.theme().background).child(scroll).children(strip.map(|strip| {
            div()
                .id("entry-confirm-footer")
                .test_support()
                .flex_shrink_0()
                .w_full()
                .border_t_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .child(div().mx_auto().w_full().max_w(rems(38.)).pb_3().child(strip))
        }))
    }
}
