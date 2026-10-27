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
//! - `daemon_step`: no daemon yet; the user may start a local one.
//! - `node_step`: node cards and the add-node form.
//! - `account_step`: unlock, create, join and restore forms.
//! - `ceremony`: account creation with a recovery phrase, when the daemon
//!   offers it.

mod account_step;
mod ceremony;
mod daemon_step;
mod node_card;
mod node_step;

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::stepper::{Stepper, StepperItem};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, Entity, IntoElement, Render, Subscription, Window, div, rems};
use ramflux_desktop_core::{AccountId, AccountKind, NodeId, Scope};

use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{self, tr};
use crate::launch::DaemonLauncher;
use crate::root::{ConfirmHost, IntentRoot};
use crate::route::{EntryStep, Page, page_for};
use crate::settings::backup::BackupPanel;

pub use ceremony::{CeremonyEvent, CeremonyPanel, Held};
pub use daemon_step::failure_text as daemon_failure_text;
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

/// Which backup the restore path restores from (view-local).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreKind {
    /// A full backup (everything, every file): the daemon's restore jobs,
    /// checked and reviewed before anything is installed.
    Full,
    /// A key backup (identity keys and contacts only): the legacy import.
    Key,
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
        let plain = |key: &'static str, window: &mut Window, cx: &mut App| {
            i18n::localized_input(key, window, cx, |input| input)
        };
        let masked = |key: &'static str, window: &mut Window, cx: &mut App| {
            i18n::localized_input(key, window, cx, |input| input.masked(true))
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

/// How long a freshly opened account session is waited for before the page
/// says it did not load and offers a way out.
const SESSION_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// The wait for a freshly opened account session.
enum SessionWait {
    /// Not waiting.
    Idle,
    /// Waiting; the task ends the wait after [`SESSION_WAIT`].
    Loading { _timer: gpui_kit::Task<()> },
    /// The bound passed (or the session reported a failure) without data.
    Failed,
}

/// The entry page view.
pub struct EntryPage {
    store: Entity<StoreHandle>,
    root: Entity<IntentRoot>,
    launcher: Entity<DaemonLauncher>,
    sink: EventSink,
    inputs: Inputs,
    path: AccountPath,
    /// The restore kind the user chose; until then the full restore when
    /// the daemon has the job API, else the key backup.
    restore_kind: Option<RestoreKind>,
    /// The full restore flow (verify, review, install, result), shared with
    /// Settings · Backup; it needs no account.
    full_restore: Entity<BackupPanel>,
    /// Account creation with a recovery phrase, shown in place of the
    /// account forms while `ceremony_open`.
    ceremony: Entity<CeremonyPanel>,
    ceremony_open: bool,
    kind: AccountKind,
    node: Option<NodeId>,
    review_nodes: bool,
    submitted: Option<AccountId>,
    /// The store's failure count when a node was last looked up; a node
    /// lookup failure after it is shown under the add-node form.
    node_lookup: Option<u64>,
    /// Where the wait for a freshly opened account session is (see
    /// [`EntryPage::awaits_session`]).
    session_wait: SessionWait,
    _subscriptions: Vec<Subscription>,
}

impl EntryPage {
    /// An entry page reading `store`, emitting through `root`, and
    /// offering to start a local daemon through `launcher`.
    pub fn new(
        store: Entity<StoreHandle>,
        root: Entity<IntentRoot>,
        launcher: Entity<DaemonLauncher>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let inputs = Inputs::new(window, cx);
        let full_restore =
            cx.new(|cx| BackupPanel::standalone(store.clone(), root.clone(), window, cx));
        let ceremony = cx.new(|cx| CeremonyPanel::new(store.clone(), root.clone(), window, cx));
        let subscriptions = vec![
            cx.observe(&full_restore, |_, _, cx| cx.notify()),
            cx.observe(&ceremony, |_, _, cx| cx.notify()),
            cx.subscribe(&ceremony, |this, _, event: &CeremonyEvent, cx| {
                let CeremonyEvent::Closed = event;
                this.ceremony_open = false;
                cx.notify();
            }),
            cx.subscribe(&store, |this, _, event: &StoreEvent, cx| {
                if let StoreEvent::Changed(changes) = event
                    && changes.touches_status()
                {
                    this.submitted = None;
                    cx.notify();
                }
                this.sync_session_wait(cx);
            }),
            cx.observe(&root, |_, _, cx| cx.notify()),
            cx.observe(&launcher, |_, _, cx| cx.notify()),
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
            launcher,
            inputs,
            path: AccountPath::Create,
            restore_kind: None,
            full_restore,
            ceremony,
            ceremony_open: false,
            kind: AccountKind::HumanInteractive,
            node: None,
            review_nodes: false,
            submitted: None,
            node_lookup: None,
            session_wait: SessionWait::Idle,
            _subscriptions: subscriptions,
        }
    }

    /// The unlock passphrase input.
    pub fn passphrase(&self) -> &Entity<InputState> {
        &self.inputs.passphrase
    }

    /// The full restore flow's panel.
    pub fn full_restore(&self) -> &Entity<BackupPanel> {
        &self.full_restore
    }

    /// The account creation with a recovery phrase.
    pub fn ceremony(&self) -> &Entity<CeremonyPanel> {
        &self.ceremony
    }

    /// Whether the ceremony screens are shown instead of the account forms.
    pub fn ceremony_open(&self) -> bool {
        self.ceremony_open
    }

    /// Whether the page holds a full restore's result (or its progress) in
    /// view: the shell keeps showing the entry page then, even when the
    /// daemon already reopened the restored account, until the user
    /// closes it.
    pub fn holds_restore(&self, cx: &App) -> bool {
        self.full_restore.read(cx).holds_result(cx)
    }

    /// Whether the restore result is held only until the store's first jobs
    /// listing (the account session that took over is still empty): the
    /// page then stays on the account step, whatever the empty store says.
    pub fn awaits_restore_listing(&self, cx: &App) -> bool {
        self.full_restore.read(cx).awaits_listing(cx)
    }

    /// Whether the account session that was just opened (an unlock) has
    /// received nothing yet while the daemon itself is reachable: its empty
    /// store would route to the daemon step, which is not where the user
    /// is. The account step shows a neutral loading line until the first
    /// data arrives.
    pub fn awaits_session(&self, cx: &App) -> bool {
        let handle = self.store.read(cx);
        handle.is_live()
            && !handle.daemon_unreachable()
            && matches!(page_for(handle.store()), Page::Entry(EntryStep::Daemon))
    }

    /// Whether the wait for the account session has run out: the bound
    /// passed, or the session reported a failure, and still no data.
    pub fn session_failed(&self, cx: &App) -> bool {
        self.awaits_session(cx)
            && (matches!(self.session_wait, SessionWait::Failed)
                || !self.store.read(cx).store().failures().is_empty())
    }

    /// Starts or ends the bounded wait as the store's state changes.
    fn sync_session_wait(&mut self, cx: &mut Context<Self>) {
        if !self.awaits_session(cx) {
            self.session_wait = SessionWait::Idle;
            return;
        }
        if matches!(self.session_wait, SessionWait::Idle) {
            self.session_wait = SessionWait::Loading {
                _timer: cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(SESSION_WAIT).await;
                    let _ = this.update(cx, |this, cx| {
                        if matches!(this.session_wait, SessionWait::Loading { .. }) {
                            this.session_wait = SessionWait::Failed;
                            cx.notify();
                        }
                    });
                }),
            };
        }
    }

    /// Opens the account session again and waits for it anew.
    pub fn retry_session(&mut self, cx: &mut Context<Self>) {
        self.session_wait = SessionWait::Idle;
        self.store.update(cx, |handle, cx| handle.reload_active_session(cx));
        self.sync_session_wait(cx);
        cx.notify();
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
        // A restore waiting for the session that took over to list its
        // jobs: that session's store is empty, so its own step would be an
        // earlier one (the daemon step).
        if self.awaits_restore_listing(cx) || self.awaits_session(cx) {
            return EntryStep::Account;
        }
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
