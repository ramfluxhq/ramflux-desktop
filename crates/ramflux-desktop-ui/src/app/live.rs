// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Live mode: the store handle on a real daemon through the SDK source.
//!
//! The daemon session's store is the truth about accounts: an account it
//! reports unlocked gets a session of its own (fed by that account's
//! source) and becomes the rendered account when none is; an account it no
//! longer reports unlocked loses its session (its store, and with it every
//! cached message, is dropped). Intents are decided against the rendered
//! store; lock (and every entry intent while no account is rendered) is
//! performed by the daemon session, everything else by the account's.

use gpui_kit::Context;
use ramflux_desktop_core::{AccountId, Intent, IntentKind, LockState, methods};
use ramflux_desktop_sdk::{DaemonStop, SdkHub};

use super::{
    AccountSession, DispatchOutcome, InspectorEntry, StoreEvent, StoreHandle, effect_label,
};

/// Label of the daemon session (shown by the developer panel).
pub const DAEMON_SESSION: &str = "rfd";

/// The hub and the daemon session.
pub(super) struct LiveLink {
    hub: SdkHub,
    daemon: AccountSession,
}

impl LiveLink {
    /// The daemon session.
    pub(super) fn daemon(&self) -> &AccountSession {
        &self.daemon
    }

    /// The recovery phrase the daemon session's source handed out, once.
    pub(super) fn take_daemon_recovery_phrase(
        &mut self,
    ) -> Option<ramflux_desktop_core::RecoveryPhrase> {
        self.daemon.take_recovery_phrase()
    }
}

impl StoreHandle {
    /// Switches to live mode on `hub`: every scenario session is dropped and
    /// the daemon session starts; account sessions follow the daemon.
    pub fn attach_live(&mut self, hub: SdkHub, cx: &mut Context<Self>) {
        self.sessions.clear();
        self.active = None;
        self.inspector.clear();
        let daemon = AccountSession::live(DAEMON_SESSION, Box::new(hub.daemon_source()));
        self.live = Some(LiveLink { hub, daemon });
        self.emit_reset(cx);
    }

    /// The daemon socket in live mode.
    pub fn live_socket(&self) -> Option<&str> {
        self.live.as_ref().map(|link| link.hub.socket())
    }

    /// Asks the daemon on the live socket to stop gracefully (`daemon.stop`);
    /// `None` when the window plays scenarios. A local application command
    /// of the launcher, not a domain intent.
    pub fn request_daemon_stop(&self) -> Option<DaemonStop> {
        self.live.as_ref().map(|link| link.hub.stop_daemon())
    }

    pub(super) fn advance_live(&mut self, ms: u64, cx: &mut Context<Self>) {
        let Some(link) = self.live.as_mut() else { return };
        let applied = link.daemon.advance(ms);
        let status = link.daemon.store().source_status();
        // The ceremony of an AI participant, the accounts the daemon lists
        // and the credential exports are the daemon session's, also while
        // an account is open (its settings show them).
        let daemon_part = applied.changes.daemon_part();
        if let (Some(part), Some(_)) = (daemon_part, self.active.as_ref()) {
            cx.emit(StoreEvent::Changed(std::rc::Rc::new(part)));
        }
        if self.active.is_none() {
            self.publish(applied, cx);
        } else if self.daemon_status.is_some_and(|seen| seen != status) {
            // The daemon's connection changed while an account session is
            // shown: routing follows it (an entry page held for a restore
            // lets go when the daemon is gone).
            let changes = ramflux_desktop_core::Changes {
                source_status: true,
                ..ramflux_desktop_core::Changes::default()
            };
            let mut set = super::ChangeSet::default();
            set.merge(changes);
            cx.emit(super::StoreEvent::Changed(std::rc::Rc::new(set)));
        }
        self.daemon_status = Some(status);
        self.reconcile_live(cx);
    }

    /// Opens the rendered account's session again (a live daemon only): a
    /// fresh source, a fresh snapshot, an empty store until it answers.
    pub fn reload_active_session(&mut self, cx: &mut Context<Self>) {
        let (Some(id), true) = (self.active.clone(), self.live.is_some()) else { return };
        // The old source goes first: it detaches by account, which must not
        // reach the scope the new source attaches.
        self.sessions.remove(&id);
        if let Some(link) = self.live.as_ref() {
            let source = link.hub.account_source(id.clone());
            self.sessions
                .insert(id.clone(), AccountSession::live(id.to_string(), Box::new(source)));
        }
        self.emit_reset(cx);
    }

    fn reconcile_live(&mut self, cx: &mut Context<Self>) {
        let Some(link) = self.live.as_ref() else { return };
        let unlocked: Vec<AccountId> = link
            .daemon
            .store()
            .accounts()
            .filter(|a| a.lock_state == LockState::Unlocked)
            .map(|a| a.id.clone())
            .collect();
        let stale: Vec<AccountId> =
            self.sessions.keys().filter(|id| !unlocked.contains(id)).cloned().collect();
        let fresh: Vec<AccountId> =
            unlocked.iter().filter(|id| !self.sessions.contains_key(*id)).cloned().collect();
        if stale.is_empty() && fresh.is_empty() {
            return;
        }
        for id in stale {
            self.sessions.remove(&id);
        }
        for id in fresh {
            let source = link.hub.account_source(id.clone());
            self.sessions
                .insert(id.clone(), AccountSession::live(id.to_string(), Box::new(source)));
        }
        let active_gone = self.active.as_ref().is_some_and(|id| !self.sessions.contains_key(id));
        if self.active.is_none() || active_gone {
            self.active = self.sessions.keys().next().cloned();
        }
        self.emit_reset(cx);
    }

    pub(super) fn dispatch_live(
        &mut self,
        intent: Intent,
        cx: &mut Context<Self>,
    ) -> DispatchOutcome {
        // The entry page's ceremonies belong to the daemon's session, also
        // once the account one of them opened is rendered.
        let ceremony = is_ceremony(&intent.kind);
        let Some(mut session) = self.session() else { return DispatchOutcome::NoAccount };
        if ceremony && let Some(link) = self.live.as_ref() {
            session = link.daemon();
        }
        // An account whose own session has not loaded yet is known to the
        // daemon's: locking it is decided there (and goes there anyway).
        if let IntentKind::LockAccount { account_id } = &intent.kind
            && session.store().account(account_id).is_none()
            && let Some(link) = self.live.as_ref()
        {
            session = link.daemon();
        }
        let (decided, records) = session.decide(intent);
        for record in records {
            self.inspector.push(record);
        }
        let effects = match decided {
            Ok(effects) => effects,
            Err(outcome) => {
                cx.emit(StoreEvent::Inspected);
                return outcome;
            }
        };
        let mut labels = Vec::new();
        for effect in effects {
            let label = effect_label(&effect);
            self.inspector.push(InspectorEntry::Effect { label: label.clone() });
            labels.push(label);
            let to_daemon =
                ceremony || self.active.is_none() || effect.method() == Some(methods::ACCOUNT_LOCK);
            if to_daemon {
                if let Some(link) = self.live.as_mut() {
                    link.daemon.submit(effect);
                }
            } else if let Some(session) = self.active_session_mut() {
                session.submit(effect);
            }
        }
        cx.emit(StoreEvent::Inspected);
        DispatchOutcome::Submitted(labels)
    }
}

/// Whether `kind` belongs to the creation or recovery ceremony of the entry
/// page, or to an AI participant's creation and credential export (which
/// are decided against the daemon's accounts and run on the daemon scope,
/// on connections of their own).
fn is_ceremony(kind: &IntentKind) -> bool {
    matches!(
        kind,
        IntentKind::BeginAgentCeremony { .. }
            | IntentKind::ConfirmAgentCeremony { .. }
            | IntentKind::ExportAgentCredentials { .. }
            | IntentKind::BeginCeremony { .. }
            | IntentKind::ShowRecoveryPhrase
            | IntentKind::HideRecoveryPhrase
            | IntentKind::ConfirmCeremony { .. }
            | IntentKind::CancelCeremony
            | IntentKind::BeginRecovery { .. }
            | IntentKind::ConfirmRecovery { .. }
            | IntentKind::CancelRecovery
    )
}
