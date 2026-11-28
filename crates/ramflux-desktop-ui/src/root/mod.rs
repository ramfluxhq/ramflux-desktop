// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The intent root: the one place UI events become intents (rules B2, B4).
//!
//! Every page (entry, workspace, settings) and every dock, palette and moment
//! emits through an [`EventSink`] whose chain ends here. The root maps the
//! event with [`intent_for`], hands the intent to the [`StoreHandle`] and,
//! when policy asks for the attended human's confirmation, keeps the pending
//! intent and opens the inline confirm strip. Only a press of its
//! Confirm button re-submits it, with `confirmed_by` set and `proposed_by` unchanged.
//!
//! Secret-carrying intents (unlock, create, activate, restore, passphrase
//! rotation) cannot travel in UI events; their forms call
//! [`IntentRoot::submit_secret`], which takes the same policy path.
//!
//! A pending confirmation is bound to what the user reviewed. When the store
//! changes under it (the surface was patched, the grant was decided or
//! failed), the root cancels it before it can be confirmed; a surface change
//! is reported ([`RootEvent::SurfaceChanged`]) so the row asks the user to
//! review it again. Policy re-checks the bound version on submit as well.

pub(crate) mod intent_map;

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Subscription, Task, Window};

use ramflux_desktop_core::{
    AccountId, AppLockPhase, ConfirmationIssuer, GrantState, Intent, IntentError, IntentKind,
    LockState, MomentId, ParticipantId, Scope, Store, UiEvent,
};

use crate::app::{
    DispatchOutcome, InspectorEntry, StoreEvent, StoreHandle, UiOutcome, audit_label, format_path,
    format_scope, intent_name, ui_event_name,
};
use crate::bubble::EventSink;
use crate::confirm::{ConfirmEvent, ConfirmRequest, ConfirmStrip};

pub use intent_map::{intent_for, step_block_id};

/// How long the window must keep its focus state before the app lock is told.
const LIFECYCLE_SETTLE: Duration = Duration::from_millis(200);

/// The longest a flurry of focus changes may delay the report.
const LIFECYCLE_SETTLE_CAP: Duration = Duration::from_secs(1);

/// The start of the result of a submitted intent.
pub const SUBMITTED: &str = "submitted";

/// The result of an intent held for the owner's confirmation.
pub const CONFIRMATION_REQUIRED: &str = "confirmation required";

/// Which page shows the confirm strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmHost {
    /// Above the composer.
    Workspace,
    /// Under the node cards of the entry page.
    Entry,
    /// At the foot of the settings overlay.
    Settings,
}

/// Reported to the pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootEvent {
    /// The strip closed (confirmed or cancelled); the host restores focus.
    ConfirmClosed,
    /// A pending confirmation went away without being submitted (declined,
    /// or cancelled because the page changed). Reported before
    /// [`RootEvent::ConfirmClosed`] when the strip closes on a decline.
    ConfirmDeclined,
    /// An action on this surface was not performed because the surface
    /// changed after the user reviewed it.
    SurfaceChanged(MomentId),
    /// "Review" was pressed on this contact's changed identity: open the
    /// identity change sheet.
    ReviewIdentity(ramflux_desktop_core::ParticipantId),
    /// A panel or dialog of the MLS groups was asked for: the workspace
    /// opens it.
    MlsPanel(ramflux_desktop_core::mls_group::MlsPanel),
}

/// Owns the pending intent and the confirm strip.
/// How long the window may stay away before the cards are suspended.
const CARD_IDLE_SUSPEND: std::time::Duration = std::time::Duration::from_secs(60);

pub struct IntentRoot {
    store: Entity<StoreHandle>,
    confirm: Entity<ConfirmStrip>,
    pending: Option<Intent>,
    host: ConfirmHost,
    /// The only issuer of human confirmations in the application. Not
    /// `Clone`; it mints one only when the strip reports a press of Confirm.
    issuer: ConfirmationIssuer,
    /// Accounts covered since the window came back, whose report is not
    /// settled yet.
    covered: BTreeSet<AccountId>,
    /// The wait that settles rapid focus changes into one report.
    settle: Option<Task<()>>,
    /// When the first change of the flurry being settled happened.
    settle_since: Option<Instant>,
    /// The wait after which the cards are suspended, while the window is away.
    card_idle: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<RootEvent> for IntentRoot {}

impl IntentRoot {
    /// The root sink every page and view emits through.
    pub fn sink(root: &Entity<Self>) -> EventSink {
        let handler = root.downgrade();
        let observer = root.downgrade();
        EventSink::root(
            move |event, window, cx| {
                handler.update(cx, |root, cx| root.on_ui_event(event, window, cx)).ok();
            },
            move |event, scope, cx| {
                observer.update(cx, |root, cx| root.record_consumed(event, scope, cx)).ok();
            },
        )
    }

    /// The confirm strip.
    pub fn confirm_strip(&self) -> &Entity<ConfirmStrip> {
        &self.confirm
    }

    /// The intent waiting for the human's confirmation.
    pub fn pending_intent(&self) -> Option<&Intent> {
        self.pending.as_ref()
    }

    /// Which page shows the strip.
    pub fn host(&self) -> ConfirmHost {
        self.host
    }

    /// Moves the strip to `host`. A pending request is cancelled when the
    /// page that showed it goes away, so it can never be confirmed unseen.
    pub fn set_host(&mut self, host: ConfirmHost, cx: &mut Context<Self>) {
        if self.host == host {
            return;
        }
        self.host = host;
        if let Some(intent) = self.pending.take() {
            self.confirm.update(cx, ConfirmStrip::close);
            self.record_intent(&intent, "confirmation cancelled: page changed", cx);
            cx.emit(RootEvent::ConfirmDeclined);
        }
        cx.notify();
    }

    /// Cancels a pending gesture before navigation can hide its reviewed facts.
    pub fn cancel_for_navigation(&mut self, cx: &mut Context<Self>) {
        if let Some(intent) = self.pending.take() {
            self.confirm.update(cx, ConfirmStrip::close);
            self.record_intent(&intent, "confirmation cancelled: navigation changed", cx);
            cx.emit(RootEvent::ConfirmDeclined);
            cx.notify();
        }
    }

    /// Submits an intent proposed by an agent participant. It follows the
    /// same policy as human intents and never carries a confirmation (R6).
    pub fn submit_agent(
        &mut self,
        agent: ParticipantId,
        kind: IntentKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        self.submit(Intent::agent(agent, kind), window, cx)
    }

    /// Submits a human intent that carries a one-shot secret. The secret
    /// goes straight to the store handle; the inspector sees only the kind.
    pub fn submit_secret(
        &mut self,
        kind: IntentKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        self.submit(Intent::human(kind), window, cx)
    }

    /// Runs policy and either submits, opens the confirmation strip, or
    /// reports the refusal. Returns a short result for the inspector.
    pub fn submit(
        &mut self,
        intent: Intent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        let request = ConfirmRequest::describe(self.store.read(cx).store(), &intent);
        match self.store.update(cx, |store, cx| store.dispatch(intent, cx)) {
            DispatchOutcome::Submitted(labels) => {
                format!("{SUBMITTED} {}", labels.join(", "))
            }
            DispatchOutcome::NeedsConfirmation(intent) => {
                // An install is confirmed for the file the sheet names; the
                // runtime resolves it again and refuses a different one.
                let mut intent = *intent;
                if let IntentKind::AgentRuntime { confirmed_path, .. } = &mut intent.kind {
                    *confirmed_path = request.runtime_path().map(str::to_owned);
                }
                if let Some(replaced) = self.pending.replace(intent) {
                    self.record_intent(&replaced, "confirmation replaced", cx);
                }
                self.confirm.update(cx, |strip, cx| strip.open(request, window, cx));
                cx.notify();
                CONFIRMATION_REQUIRED.to_owned()
            }
            DispatchOutcome::Refused(error) => {
                if let IntentError::SurfaceChanged(moment) = &error {
                    cx.emit(RootEvent::SurfaceChanged(moment.clone()));
                }
                format!("refused: {error}")
            }
            DispatchOutcome::NoAccount => "no account".to_owned(),
        }
    }

    fn on_ui_event(&mut self, event: UiEvent, window: &mut Window, cx: &mut Context<Self>) {
        let started = Instant::now();
        let path = format_path(&event.scope_path);
        let kind = ui_event_name(&event);
        if let ramflux_desktop_core::UiEventKind::ReviewIdentityPressed { participant_id } =
            &event.kind
        {
            // Navigation, not an intent: the workspace opens the sheet.
            cx.emit(RootEvent::ReviewIdentity(participant_id.clone()));
        }
        if let ramflux_desktop_core::UiEventKind::MlsPanelRequested { panel } = &event.kind {
            // Navigation, not an intent: the workspace opens the panel.
            cx.emit(RootEvent::MlsPanel(panel.clone()));
        }
        let intent = intent_for(&event, self.store.read(cx).store()).map(Intent::human);
        let outcome = match intent {
            None => UiOutcome::Ignored,
            Some(intent) => {
                let name = intent_name(&intent).to_owned();
                // The picked mentions go with a post the policy accepted; a
                // refused send keeps them.
                let sent_in = match &intent.kind {
                    IntentKind::MlsGroupSend { group_id, retry_of: None, .. } => {
                        Some(group_id.clone())
                    }
                    _ => None,
                };
                let result = self.submit(intent, window, cx);
                if let Some(group_id) = sent_in
                    && result.starts_with(SUBMITTED)
                {
                    self.store.read(cx).store().mls_mentions_spent(&group_id);
                }
                UiOutcome::Intent { intent: name, result }
            }
        };
        let entry = InspectorEntry::UiEvent { path, kind, outcome, elapsed: started.elapsed() };
        self.store.update(cx, |store, cx| store.record(entry, cx));
    }

    fn record_consumed(&mut self, event: &UiEvent, scope: &Scope, cx: &mut Context<Self>) {
        let entry = InspectorEntry::UiEvent {
            path: format_path(&event.scope_path),
            kind: ui_event_name(event),
            outcome: UiOutcome::Consumed { at: format_scope(scope) },
            elapsed: Duration::ZERO,
        };
        self.store.update(cx, |store, cx| store.record(entry, cx));
    }

    fn record_intent(&mut self, intent: &Intent, result: &str, cx: &mut Context<Self>) {
        let entry = InspectorEntry::Intent {
            intent: intent_name(intent),
            actor: audit_label(intent),
            result: result.to_owned(),
        };
        self.store.update(cx, |store, cx| store.record(entry, cx));
    }

    /// Cancels the pending confirmation when what it was bound to changed
    /// in the store (subscribed in [`IntentRoot::attach`]).
    fn recheck_pending(&mut self, cx: &mut Context<Self>) {
        let Some(intent) = &self.pending else { return };
        let Some((result, surface)) = stale(self.store.read(cx).store(), &intent.kind) else {
            return;
        };
        let Some(intent) = self.pending.take() else { return };
        self.confirm.update(cx, ConfirmStrip::close);
        self.record_intent(&intent, result, cx);
        cx.emit(RootEvent::ConfirmDeclined);
        if let Some(moment) = surface {
            cx.emit(RootEvent::SurfaceChanged(moment));
        }
        cx.emit(RootEvent::ConfirmClosed);
        cx.notify();
    }

    /// Handles the strip's outcome (subscribed in [`IntentRoot::attach`]).
    fn on_confirm_event(
        &mut self,
        event: ConfirmEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(intent) = self.pending.take() else { return };
        match event {
            ConfirmEvent::Confirmed => {
                let confirmed = intent.confirmed_by_human(self.issuer.issue_tap());
                let label = audit_label(&confirmed);
                let name = intent_name(&confirmed);
                let result = self.submit(confirmed, window, cx);
                let entry = InspectorEntry::Intent { intent: name, actor: label, result };
                self.store.update(cx, |store, cx| store.record(entry, cx));
            }
            ConfirmEvent::Cancelled => {
                self.record_intent(&intent, "confirmation cancelled", cx);
                cx.emit(RootEvent::ConfirmDeclined);
            }
        }
        cx.emit(RootEvent::ConfirmClosed);
        cx.notify();
    }

    /// The open accounts whose app lock is on, or not known yet, when the
    /// daemon is connected (only a connected daemon has an app lock to read).
    fn lifecycle_accounts(&self, cx: &Context<Self>) -> Option<Vec<AccountId>> {
        let handle = self.store.read(cx);
        if !handle.daemon_connected() {
            return None;
        }
        let store = handle.entry_store();
        Some(
            store
                .accounts()
                .filter(|a| a.lock_state == LockState::Unlocked)
                .filter(|a| store.app_lock(&a.id).is_none_or(|lock| lock.enabled))
                .map(|a| a.id.clone())
                .collect(),
        )
    }

    /// The window went to the background or came back to the front. Coming
    /// back, the content of the accounts whose daemon was told it went away
    /// is covered at once, until the daemon answers (the reply carries the
    /// state). The report itself is sent once the window has settled for
    /// [`LIFECYCLE_SETTLE`], so rapid focus changes send one report with
    /// the final phase, and none when it is the phase the daemon already has.
    /// A flurry of changes settles after [`LIFECYCLE_SETTLE_CAP`] at the
    /// latest.
    fn on_window_activation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Cards are suspended once the window has been away for a while, and
        // resumed when it is back (the runtime frees its VMs meanwhile).
        if window.is_window_active() {
            self.card_idle = None;
            self.store.update(cx, |store, cx| store.card_resume_all(cx));
        } else if self.card_idle.is_none() {
            self.card_idle = Some(cx.spawn_in(window, async move |root, cx| {
                cx.background_executor().timer(CARD_IDLE_SUSPEND).await;
                let _ = root.update_in(cx, |root, _, cx| {
                    root.store.update(cx, |store, cx| store.card_suspend_all(cx));
                });
            }));
        }
        if let Some(accounts) = self.lifecycle_accounts(cx)
            && window.is_window_active()
        {
            let handle = self.store.read(cx);
            let covered: Vec<AccountId> = accounts
                .into_iter()
                .filter(|id| handle.reported_phase(id) != Some(AppLockPhase::Foreground))
                .filter(|id| self.covered.insert(id.clone()))
                .collect();
            self.store.update(cx, |store, cx| store.hold_for_resume(covered, cx));
        }
        let now = cx.background_executor().now();
        let since = *self.settle_since.get_or_insert(now);
        let wait = LIFECYCLE_SETTLE
            .min(LIFECYCLE_SETTLE_CAP.saturating_sub(now.saturating_duration_since(since)));
        self.settle = Some(cx.spawn_in(window, async move |root, cx| {
            cx.background_executor().timer(wait).await;
            let _ = root.update_in(cx, |root, window, cx| root.settle_lifecycle(window, cx));
        }));
    }

    /// Reports the settled phase of the window to the app lock of each
    /// account whose daemon does not have it yet. Every account covered since
    /// the window came back either gets the foreground report (the daemon's
    /// answer gives it back) or has its hold given back here: nothing
    /// covered is left without a report to wait for.
    fn settle_lifecycle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settle_since = None;
        let phase = if window.is_window_active() {
            AppLockPhase::Foreground
        } else {
            AppLockPhase::Background
        };
        let covered = std::mem::take(&mut self.covered);
        let mut given_back: Vec<AccountId> = Vec::new();
        let Some(accounts) = self.lifecycle_accounts(cx) else {
            // The daemon is not there to answer.
            given_back.extend(covered);
            self.store.update(cx, |store, cx| store.cancel_resume_hold(&given_back, cx));
            return;
        };
        self.store.update(cx, |store, _| store.retain_reported(&accounts));
        given_back.extend(covered.iter().filter(|id| !accounts.contains(id)).cloned());
        for account_id in accounts {
            let held = covered.contains(&account_id);
            let has = self.store.read(cx).reported_phase(&account_id);
            if !report_due(has, phase) {
                if held {
                    given_back.push(account_id);
                }
                continue;
            }
            if held && phase == AppLockPhase::Background {
                given_back.push(account_id.clone());
            }
            let kind = IntentKind::AppLockLifecycle { account_id: account_id.clone(), phase };
            let outcome =
                self.store.update(cx, |store, cx| store.dispatch(Intent::human(kind), cx));
            if held
                && phase == AppLockPhase::Foreground
                && !matches!(outcome, DispatchOutcome::Submitted(_))
            {
                // The report was not sent: no answer will come.
                given_back.push(account_id);
            }
        }
        if !given_back.is_empty() {
            self.store.update(cx, |store, cx| store.cancel_resume_hold(&given_back, cx));
        }
    }

    /// Asks for the reads the MLS state needs next (a gap fill, the group
    /// list after a join, the info of a group after a membership or settings
    /// event). They are reads, each asked once per need: the source's
    /// started fact marks them asked. Nothing is sent or resent here.
    fn mls_followups(&mut self, cx: &mut Context<Self>) {
        let (now, kinds) = {
            let handle = self.store.read(cx);
            let Some(session) = handle.session() else { return };
            let now = session.now();
            (now, session.store().mls_followups(now))
        };
        let _ = now;
        for kind in kinds {
            self.store.update(cx, |store, cx| store.dispatch(Intent::human(kind), cx));
        }
    }

    /// Creates a root dispatching to `store` and wires the strip's events,
    /// which need a window.
    pub fn attach(store: Entity<StoreHandle>, window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| {
            let confirm = cx.new(ConfirmStrip::new);
            let subscriptions = vec![
                cx.subscribe_in(
                    &confirm,
                    window,
                    |root: &mut Self, _, event: &ConfirmEvent, window, cx| {
                        root.on_confirm_event(*event, window, cx);
                    },
                ),
                cx.subscribe(&store, |root: &mut Self, _, event: &StoreEvent, cx| {
                    if let StoreEvent::Changed(changes) = event {
                        root.recheck_pending(cx);
                        if !changes.domain().mls.is_empty() {
                            root.mls_followups(cx);
                        }
                    }
                }),
                cx.observe_window_activation(window, |root: &mut Self, window, cx| {
                    root.on_window_activation(window, cx);
                }),
            ];
            Self {
                store,
                confirm,
                pending: None,
                host: ConfirmHost::Workspace,
                issuer: ConfirmationIssuer::new(),
                covered: BTreeSet::new(),
                settle: None,
                settle_since: None,
                card_idle: None,
                _subscriptions: subscriptions,
            }
        })
    }
}

/// Whether the daemon has to be told the window is in `phase`: it is unless
/// its last answer already says so. This holds for an account that was not
/// covered as well: a stored background (or no answer yet) while the window
/// is in use would lock the account under the user's hands.
fn report_due(has: Option<AppLockPhase>, phase: AppLockPhase) -> bool {
    has != Some(phase)
}

/// Why a pending intent no longer matches the store: the inspector result,
/// and the surface moment when a surface changed. `None` while it matches.
fn stale(store: &Store, kind: &IntentKind) -> Option<(&'static str, Option<MomentId>)> {
    match kind {
        IntentKind::SurfaceAction { moment_id, surface_hash, .. } => {
            let current = store.surface_hash(moment_id) == Some(surface_hash);
            (!current).then(|| ("confirmation cancelled: surface changed", Some(moment_id.clone())))
        }
        IntentKind::Approve { grant_id } => {
            let open = store
                .grant(grant_id)
                .is_some_and(|g| g.state == GrantState::Pending && g.failure.is_none());
            (!open).then_some(("confirmation cancelled: request closed", None))
        }
        IntentKind::Contact { participant_id, action } => {
            let current =
                ramflux_desktop_core::contact_action_current(store, participant_id, *action);
            (!current).then_some(("confirmation cancelled: contact changed", None))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_foreground_is_reported_when_the_daemon_has_background_or_nothing() {
        assert!(report_due(Some(AppLockPhase::Background), AppLockPhase::Foreground));
        assert!(report_due(None, AppLockPhase::Foreground));
        assert!(!report_due(Some(AppLockPhase::Foreground), AppLockPhase::Foreground));
    }

    #[test]
    fn the_background_is_reported_unless_the_daemon_has_it() {
        assert!(report_due(Some(AppLockPhase::Foreground), AppLockPhase::Background));
        assert!(report_due(None, AppLockPhase::Background));
        assert!(!report_due(Some(AppLockPhase::Background), AppLockPhase::Background));
    }
}
