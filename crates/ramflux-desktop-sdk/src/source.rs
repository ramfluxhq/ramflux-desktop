// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! [`SdkSource`]: the core `Source` over the bridge, and [`SdkHub`] that
//! hands one out per scope.
//!
//! The source assigns each delivered fact the next sequence number of its
//! scope (deterministic per account: one counter, facts in bridge order) and
//! keeps the counter across reconnects, so the store's cursor keeps moving
//! forward when the bridge re-snapshots. It never blocks: `submit` and
//! `focus` queue a command, `drain` takes what has arrived.

use std::sync::mpsc as std_mpsc;

use ramflux_desktop_core::{AccountId, ContextId, Effect, EventEnvelope, Source, Timestamp};

use crate::bridge::{Bridge, Command, Delivery};
use crate::config::SdkConfig;
use crate::translate::Scope;

/// Starts the bridge and creates sources for its scopes.
#[derive(Clone)]
pub struct SdkHub {
    bridge: Bridge,
    socket: String,
}

impl SdkHub {
    /// Starts the bridge thread for `config`.
    ///
    /// # Errors
    ///
    /// Returns the I/O error when the bridge thread cannot start.
    pub fn start(config: SdkConfig) -> std::io::Result<Self> {
        let socket = config.socket_path.to_string_lossy().into_owned();
        Ok(Self { bridge: Bridge::start(config)?, socket })
    }

    /// The daemon socket the hub attaches to.
    pub fn socket(&self) -> &str {
        &self.socket
    }

    /// A source for the daemon scope (status, nodes, accounts; the entry page).
    pub fn daemon_source(&self) -> SdkSource {
        SdkSource::attach(self.bridge.clone(), Scope::Daemon)
    }

    /// A source for one unlocked account.
    pub fn account_source(&self, account: AccountId) -> SdkSource {
        SdkSource::attach(self.bridge.clone(), Scope::Account(account))
    }
}

/// The SDK-backed source of one scope.
pub struct SdkSource {
    bridge: Bridge,
    scope: Scope,
    inbox: std_mpsc::Receiver<Delivery>,
    last_seq: u64,
}

impl std::fmt::Debug for SdkSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SdkSource")
            .field("scope", &self.scope)
            .field("last_seq", &self.last_seq)
            .finish()
    }
}

impl SdkSource {
    fn attach(bridge: Bridge, scope: Scope) -> Self {
        let (outbox, inbox) = std_mpsc::channel();
        bridge.send(Command::Attach { scope: scope.clone(), outbox });
        Self { bridge, scope, inbox, last_seq: 0 }
    }

    /// The scope served.
    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    /// The last sequence number handed out (the scope's cursor).
    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }
}

impl Source for SdkSource {
    fn submit(&mut self, effect: Effect) {
        self.bridge.send(Command::Effect { scope: self.scope.clone(), effect });
    }

    fn drain(&mut self, _now: Timestamp) -> Vec<EventEnvelope> {
        let mut envelopes = Vec::new();
        while let Ok(delivery) = self.inbox.try_recv() {
            self.last_seq += 1;
            envelopes.push(EventEnvelope::new(self.last_seq, delivery.at, delivery.event));
        }
        envelopes
    }

    fn focus(&mut self, context: Option<&ContextId>) {
        self.bridge.send(Command::Focus { scope: self.scope.clone(), context: context.cloned() });
    }
}

impl Drop for SdkSource {
    fn drop(&mut self) {
        self.bridge.send(Command::Detach { scope: self.scope.clone() });
    }
}
