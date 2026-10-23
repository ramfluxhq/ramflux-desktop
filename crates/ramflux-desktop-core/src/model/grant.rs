// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Risk, grants, standing approvals and the audit trail.

use serde::{Deserialize, Serialize};

use super::actor::Actor;
use crate::ids::{GrantId, StandingApprovalId, Timestamp};

/// Risk level of a grant or action. Ordered from lowest to highest.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum Risk {
    /// May execute if policy allows.
    #[default]
    Low,
    /// Audit recommended.
    Medium,
    /// Explicit confirmation required.
    High,
    /// High rules plus stronger future hooks.
    Critical,
}

impl Risk {
    /// Returns true for `High` and `Critical`.
    pub const fn requires_confirmation(self) -> bool {
        matches!(self, Self::High | Self::Critical)
    }
}

/// Grant decision state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GrantState {
    /// Awaiting a decision.
    #[default]
    Pending,
    /// Approved.
    Approved,
    /// Denied.
    Denied,
    /// Expired before a decision.
    Expired,
    /// Approved, then revoked before use.
    Revoked,
}

/// Why the backend refused a grant decision, or a grant stopped being
/// usable. A failed grant is closed: it is never approved or retried from
/// the client; the agent has to request it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GrantFailure {
    /// The single-use approval was already used or decided elsewhere.
    AlreadyUsed,
    /// The grant expired before it could be used.
    Expired,
    /// The arguments bound by the backend differ from the ones reviewed.
    ArgumentsMismatch,
    /// A decision was sent again for an approval already decided here.
    Replay,
    /// The backend rejected the approval signature.
    SignatureRejected,
    /// The request needs an approval signed on the approving device.
    SignedApprovalRequired,
    /// The backend invalidated the grant (revoked, or its tool set changed).
    Invalidated,
}

/// An authorization grant bound to exact arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// Grant id.
    pub id: GrantId,
    /// Scope, e.g. a tool name.
    pub scope: String,
    /// Effective risk.
    pub risk: Risk,
    /// Consumed on first use.
    #[serde(default)]
    pub single_use: bool,
    /// Hash of the bound arguments.
    pub args_hash: String,
    /// Expiry.
    #[serde(default)]
    pub expiry: Option<Timestamp>,
    /// Decision state.
    #[serde(default)]
    pub state: GrantState,
    /// Set when the backend refused it (fail closed); never cleared.
    #[serde(default)]
    pub failure: Option<GrantFailure>,
}

/// A standing approval: a bounded, pre-authorized policy for repeated requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandingApproval {
    /// Id.
    pub id: StandingApprovalId,
    /// Scope it covers, e.g. a tool name.
    pub scope: String,
    /// Highest risk it may approve.
    pub max_risk: Risk,
    /// Expiry.
    #[serde(default)]
    pub expiry: Option<Timestamp>,
    /// Times it has been used.
    #[serde(default)]
    pub uses: u32,
    /// Revoked; kept for the record.
    #[serde(default)]
    pub revoked: bool,
}

/// One read-only audit record (approvals, reveals, node trust, revocations).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    /// When.
    pub at: Timestamp,
    /// Contract method that was performed.
    pub action: String,
    /// What it applied to, as an id.
    pub subject: String,
    /// Who proposed it.
    pub proposed_by: Actor,
    /// Whether the attended human confirmed it with a hold.
    #[serde(default)]
    pub human_confirmed: bool,
}
