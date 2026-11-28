// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Identity of a direct-chat peer and the security of a direct conversation.
//!
//! An identity is `Verified` or `Tofu` (seen first on this device) until it
//! changes; a changed identity is accepted only by the user, in a sheet. The
//! conversation's security says whether its encryption is set up, was
//! re-established, and how many devices the peer has.

use serde::{Deserialize, Serialize};

use crate::ids::Timestamp;

/// The state of a contact's identity (`identity_state`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentityState {
    /// Seen first on this device; no badge. Also what a state the SDK did
    /// not name renders as.
    #[default]
    Tofu,
    /// Verified.
    Verified,
    /// The identity changed and the user has not accepted the change.
    Changed,
}

impl IdentityState {
    /// Whether the identity change waits for the user.
    pub const fn is_changed(self) -> bool {
        matches!(self, Self::Changed)
    }
}

/// An identity change the user has not decided on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityChange {
    /// The anchor the contact had.
    pub previous_anchor: String,
    /// The anchor the contact has now: what an acceptance names.
    pub new_anchor: String,
    /// The contact's @username as the event named it, when it did.
    #[serde(default)]
    pub username: Option<String>,
    /// When the change was seen.
    pub seen_at: Timestamp,
}

/// The state of a direct conversation's encryption.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecurityState {
    /// End-to-end encrypted.
    #[default]
    Ok,
    /// Being set up.
    Pending,
    /// Set up again after a fork.
    ReEstablished,
}

/// A warning on a conversation's security.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecurityWarning {
    /// The encryption was re-established after a fork: some messages may
    /// not have arrived.
    ForkRecovered,
}

/// What `conversation.security` says about a direct conversation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationSecurity {
    /// The state.
    pub state: SecurityState,
    /// The epoch of the encryption.
    pub epoch: u64,
    /// How many devices of the peer take part.
    pub peer_devices: u32,
    /// Whether any member is flagged as an AI.
    pub any_ai: bool,
    /// The warning, when there is one.
    #[serde(default)]
    pub warning: Option<SecurityWarning>,
}

/// Why an acceptance did not take effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AcceptFailure {
    /// The change was already handled (`no_pending_change`).
    AlreadyHandled,
    /// The identity changed again (`anchor_mismatch`).
    ChangedAgain,
    /// Anything else.
    Failed,
}
