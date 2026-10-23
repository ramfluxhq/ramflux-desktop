// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The intent root: the one place UI events become intents (rules B2, B4).
//!
//! Every page (entry, workspace, settings) and every dock, palette and moment
//! emits through an [`EventSink`] whose chain ends here. The root maps the
//! event with [`intent_for`], hands the intent to the [`StoreHandle`] and,
//! when policy asks for the attended human's confirmation, keeps the pending
//! intent and opens the inline hold-to-confirm strip. Only a completed hold
//! re-submits it, with `confirmed_by` set and `proposed_by` unchanged.
//!
//! Secret-carrying intents (unlock, create, activate, restore, passphrase
//! rotation) cannot travel in UI events; their forms call
//! [`IntentRoot::submit_secret`], which takes the same policy path.
//!
//! A pending confirmation is bound to what the user reviewed. When the store
//! changes under it (the surface was patched, the grant was decided or
//! failed), the root cancels it before a hold can complete; a surface change
//! is reported ([`RootEvent::SurfaceChanged`]) so the row asks the user to
//! review it again. Policy re-checks the bound version on submit as well.

mod intent_map;

use std::time::{Duration, Instant};

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Subscription, Window};

use ramflux_desktop_core::{
    ConfirmationIssuer, GrantState, Intent, IntentError, IntentKind, MomentId, ParticipantId,
    Scope, Store, UiEvent,
};

use crate::app::{
    DispatchOutcome, InspectorEntry, StoreEvent, StoreHandle, UiOutcome, audit_label, format_path,
    format_scope, intent_name, ui_event_name,
};
use crate::bubble::EventSink;
use crate::confirm::{ConfirmEvent, ConfirmRequest, ConfirmStrip};

pub use intent_map::{intent_for, step_block_id};

/// Which page shows the hold-to-confirm strip.
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
    /// The strip closed (held or cancelled); the host restores focus.
    ConfirmClosed,
    /// An action on this surface was not performed because the surface
    /// changed after the user reviewed it.
    SurfaceChanged(MomentId),
}

/// Owns the pending intent and the hold-to-confirm strip.
pub struct IntentRoot {
    store: Entity<StoreHandle>,
    confirm: Entity<ConfirmStrip>,
    pending: Option<Intent>,
    host: ConfirmHost,
    /// The only issuer of human confirmations in the application. Not
    /// `Clone`; it mints one only when the strip reports a completed hold.
    issuer: ConfirmationIssuer,
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

    /// The hold-to-confirm strip.
    pub fn confirm_strip(&self) -> &Entity<ConfirmStrip> {
        &self.confirm
    }

    /// The intent waiting for the human's hold.
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
        }
        cx.notify();
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
            DispatchOutcome::Submitted(labels) => format!("submitted {}", labels.join(", ")),
            DispatchOutcome::NeedsConfirmation(intent) => {
                if let Some(replaced) = self.pending.replace(*intent) {
                    self.record_intent(&replaced, "confirmation replaced", cx);
                }
                self.confirm.update(cx, |strip, cx| strip.open(request, window, cx));
                cx.notify();
                "confirmation required".to_owned()
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
        let intent = intent_for(&event, self.store.read(cx).store()).map(Intent::human);
        let outcome = match intent {
            None => UiOutcome::Ignored,
            Some(intent) => {
                let name = intent_name(&intent).to_owned();
                let result = self.submit(intent, window, cx);
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
            ConfirmEvent::HoldCompleted { held_ms } => {
                let Some(confirmation) = self.issuer.issue_hold(held_ms) else {
                    self.record_intent(&intent, "confirmation refused: hold too short", cx);
                    cx.emit(RootEvent::ConfirmClosed);
                    cx.notify();
                    return;
                };
                let confirmed = intent.confirmed_by_human(confirmation);
                let label = audit_label(&confirmed);
                let name = intent_name(&confirmed);
                let result = self.submit(confirmed, window, cx);
                let entry = InspectorEntry::Intent { intent: name, actor: label, result };
                self.store.update(cx, |store, cx| store.record(entry, cx));
            }
            ConfirmEvent::Cancelled => self.record_intent(&intent, "confirmation cancelled", cx),
        }
        cx.emit(RootEvent::ConfirmClosed);
        cx.notify();
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
                    if matches!(event, StoreEvent::Changed(_)) {
                        root.recheck_pending(cx);
                    }
                }),
            ];
            Self {
                store,
                confirm,
                pending: None,
                host: ConfirmHost::Workspace,
                issuer: ConfirmationIssuer::new(),
                _subscriptions: subscriptions,
            }
        })
    }
}

/// Why a pending intent no longer matches the store: the inspector result,
/// and the surface moment when a surface changed. `None` while it matches.
fn stale(store: &Store, kind: &IntentKind) -> Option<(&'static str, Option<MomentId>)> {
    match kind {
        IntentKind::SurfaceAction { moment_id, surface_hash, .. } => {
            let current = store.surface_hash(moment_id) == Some(surface_hash)
                && !store.surface_conflict(moment_id);
            (!current).then(|| ("confirmation cancelled: surface changed", Some(moment_id.clone())))
        }
        IntentKind::Approve { grant_id } => {
            let open = store
                .grant(grant_id)
                .is_some_and(|g| g.state == GrantState::Pending && g.failure.is_none());
            (!open).then_some(("confirmation cancelled: request closed", None))
        }
        _ => None,
    }
}
