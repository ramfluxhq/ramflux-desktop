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
use ramflux_desktop_core::{AccountId, Intent, LockState, methods};
use ramflux_desktop_sdk::SdkHub;

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

    pub(super) fn advance_live(&mut self, ms: u64, cx: &mut Context<Self>) {
        let Some(link) = self.live.as_mut() else { return };
        let applied = link.daemon.advance(ms);
        if self.active.is_none() {
            self.publish(applied, cx);
        }
        self.reconcile_live(cx);
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
        let Some(session) = self.session() else { return DispatchOutcome::NoAccount };
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
            let to_daemon = self.active.is_none() || effect.method() == Some(methods::ACCOUNT_LOCK);
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
