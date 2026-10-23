// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! What the backend behind the source can do, what it reported about
//! itself, and effects it could not perform.
//!
//! A source that lacks a counterpart for some contract method declares it
//! in [`Capabilities`] with the reason; policy then refuses the intent and
//! the UI shows the control disabled with that reason, instead of a source
//! faking data.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Methods the backend cannot perform, each with the reason shown to the user.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Contract method name to reason.
    #[serde(default)]
    pub unsupported: BTreeMap<String, String>,
}

impl Capabilities {
    /// Why `method` is unavailable, or `None` when the backend performs it.
    pub fn unsupported_reason(&self, method: &str) -> Option<&str> {
        self.unsupported.get(method).map(String::as_str)
    }
}

/// What the local daemon reported about itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonInfo {
    /// Local socket the source is attached to.
    pub socket_path: String,
    /// Accounts the daemon has open.
    #[serde(default)]
    pub open_accounts: u32,
}

/// How the source learns about changes in the daemon.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncMode {
    /// Not known yet: the source is attaching or opening its event stream.
    #[default]
    Connecting,
    /// The daemon's event stream: changes arrive as the daemon applies them.
    Stream,
    /// Periodic reads: the event stream is unavailable and polling is on.
    Polling,
    /// No live updates: the event stream is unavailable and polling is off.
    Off,
}

/// The daemon's own background sync, as it last reported it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DaemonSyncState {
    /// The last run moved messages.
    Running,
    /// The last run found nothing new.
    Idle,
    /// The last run failed (for example, the node was unreachable).
    Error,
}

/// How one account's source is kept up to date. Never holds content.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncStatus {
    /// The live mode.
    #[serde(default)]
    pub mode: SyncMode,
    /// Why the event stream is not used (the daemon's or transport's
    /// words); `None` while it is.
    #[serde(default)]
    pub reason: Option<String>,
    /// The last daemon event applied (the stream cursor).
    #[serde(default)]
    pub cursor: Option<u64>,
    /// The daemon's background sync state.
    #[serde(default)]
    pub daemon_sync: Option<DaemonSyncState>,
    /// When the daemon's background sync last ran.
    #[serde(default)]
    pub last_run: Option<crate::ids::Timestamp>,
    /// Inbound messages the daemon holds but cannot route yet.
    #[serde(default)]
    pub unrouted: Option<u64>,
}

/// Why an effect failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureCode {
    /// The backend has no counterpart for the method.
    UnsupportedByBackend,
    /// The daemon answered with an error (its code).
    Daemon(String),
    /// The daemon could not be reached or the connection broke.
    Transport,
    /// The daemon did not answer in time.
    Timeout,
}

/// An effect the source could not perform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectFailure {
    /// Contract method of the effect.
    pub method: String,
    /// Classification.
    pub code: FailureCode,
    /// Message for the user; never secret material.
    pub message: String,
}
