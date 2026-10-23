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
