// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Accounts, the nodes they bind to, and the account's devices.

use serde::{Deserialize, Serialize};

use crate::ids::{AccountId, DeviceId, NodeId, ParticipantId, Timestamp};

/// Whether the local account is unlocked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LockState {
    /// Secrets are sealed; the user must unlock first.
    #[default]
    Locked,
    /// The account is usable.
    Unlocked,
}

/// What an account is for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountKind {
    /// A person using this desktop.
    #[default]
    HumanInteractive,
    /// A headless agent identity operated from this desktop.
    HeadlessAi,
}

/// Whether the account's outbound secure sessions are usable.
///
/// After a full restore the device activates itself again under the
/// account's root before its first send: until then sending is refused
/// (history and files stay readable). The daemon reports it in the
/// account's status and announces the change.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecureSessions {
    /// Sending works.
    #[default]
    Ready,
    /// Being re-established after a restore; sending is refused meanwhile.
    Reestablishing,
}

/// A local account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// Account id.
    pub id: AccountId,
    /// Human-readable label.
    pub label: String,
    /// What the account is for.
    #[serde(default)]
    pub kind: AccountKind,
    /// Home node, once joined.
    #[serde(default)]
    pub node_id: Option<NodeId>,
    /// This device's id, once registered.
    #[serde(default)]
    pub device_id: Option<String>,
    /// Lock state.
    #[serde(default)]
    pub lock_state: LockState,
    /// The participant that is this account in its contexts. Outbound
    /// moments (and so receipts) are the ones whose origin is this id.
    #[serde(default)]
    pub self_participant_id: Option<ParticipantId>,
    /// Whether sending works, or secure sessions are being re-established
    /// after a restore.
    #[serde(default)]
    pub secure_sessions: SecureSessions,
    /// Whether the account is registered with its node (`account.list`
    /// state): `Pending` for an account made by the recovery phrase flow
    /// until registration exists.
    #[serde(default)]
    pub networking: super::Networking,
    /// Last resident session state; custody-only answers cannot replace it.
    #[serde(default)]
    pub networking_session: Option<super::Networking>,
    /// Typed reason and retry details from the daemon.
    #[serde(default)]
    pub networking_reason: Option<NetworkingReason>,
}

/// How a node endpoint was pinned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PinSource {
    /// Shipped bootstrap list.
    Bootstrap,
    /// From an invite.
    Invite,
    /// Entered by the user and trusted on first use (TOFU).
    Manual,
}

impl PinSource {
    /// Whether trust rests only on first use and needs a sticky warning.
    pub const fn is_trust_on_first_use(self) -> bool {
        matches!(self, Self::Manual)
    }
}

/// Node health as last observed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeHealth {
    /// Not yet probed.
    #[default]
    Unknown,
    /// Reachable and serving.
    Healthy,
    /// Reachable with errors or latency.
    Degraded,
    /// Not reachable.
    Unreachable,
}

/// Result of probing one node service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeOutcome {
    /// The service answered.
    pub ok: bool,
    /// Round-trip time, when it answered.
    #[serde(default)]
    pub latency_ms: Option<u32>,
    /// Short diagnostic, e.g. an error class. Never secret material.
    #[serde(default)]
    pub detail: String,
}

/// A node probe (NODE-01 `node.probe` shape): gateway and relay health.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeProbe {
    /// When the probe ran.
    pub at: Timestamp,
    /// Gateway session open.
    pub gateway: ProbeOutcome,
    /// Relay health.
    pub relay: ProbeOutcome,
}

fn yes() -> bool {
    true
}

/// A Ramflux node profile (NODE-01).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    /// Node id.
    pub id: NodeId,
    /// Known endpoints.
    #[serde(default)]
    pub endpoints: Vec<String>,
    /// Fingerprint of the node's identity key, as shown for out-of-band comparison.
    #[serde(default)]
    pub fingerprint: Option<String>,
    /// Where the pin came from.
    pub pin_source: PinSource,
    /// Last observed health.
    #[serde(default)]
    pub health: NodeHealth,
    /// Capabilities the discovery record advertises.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Expiry of the signed discovery record.
    #[serde(default)]
    pub expires_at: Option<Timestamp>,
    /// Persisted and pinned. False while a discovered node awaits confirmation.
    #[serde(default = "yes")]
    pub confirmed: bool,
    /// Shipped with the client.
    #[serde(default)]
    pub builtin: bool,
    /// Offered for new accounts.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Default node for new accounts.
    #[serde(default)]
    pub is_default: bool,
    /// Last probe result.
    #[serde(default)]
    pub probe: Option<NodeProbe>,
}

/// One of the account's devices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    /// Device id.
    pub id: DeviceId,
    /// Owning account.
    pub account_id: AccountId,
    /// Display label.
    pub label: String,
    /// The device running this client.
    #[serde(default)]
    pub this_device: bool,
    /// Revoked; kept for the record.
    #[serde(default)]
    pub revoked: bool,
    /// Last time the device was seen by the node.
    #[serde(default)]
    pub last_seen: Option<Timestamp>,
}

/// The six daemon networking reason kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkingReasonKind {
    /// Choose a node.
    NoNodeSelected,
    /// The node cannot be reached yet.
    NodeUnreachable,
    /// The node does not offer custody.
    NodeCustodyUnavailable,
    /// The node has exhausted admission capacity.
    NodeBusy,
    /// Registration was refused with a typed code.
    RegistrationRefused,
    /// Custody must be renewed.
    CustodyExpired,
}

/// Complete refusal code, including migration details.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationRefusal {
    /// Unsupported protocol.
    Unsupported,
    /// Invalid registration input.
    InvalidInput,
    /// Authority unavailable.
    AuthorityUnavailable,
    /// Identity conflict.
    PrincipalIdTaken,
    /// Unknown challenge.
    ChallengeUnknown,
    /// Expired challenge.
    ChallengeExpired,
    /// Replayed challenge.
    Replay,
    /// Binding refused.
    BindingRefused,
    /// Session required.
    SessionRequired,
    /// Session expired.
    SessionExpired,
    /// Client or node update required.
    UpdateRequired,
    /// Admission capacity exhausted.
    Capacity,
    /// Temporary listener saturation.
    Busy,
    /// Storage temporarily unavailable.
    PersistenceUnavailable,
    /// Migration refusal with the complete protocol detail.
    HomeMigration {
        /// Typed protocol reason serialized without discarding nested fields.
        reason: serde_json::Value,
        /// Complete negative acknowledgement, if present.
        nack: Option<serde_json::Value>,
    },
}

/// Admission capacity identity and absolute reset deadline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkingCapacity {
    /// Server capacity scope.
    pub scope: String,
    /// Absolute server deadline.
    pub resets_at_unix: u64,
}

/// Networking explanation shared by status snapshots and events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkingReason {
    /// Reason kind.
    pub kind: NetworkingReasonKind,
    /// Registration refusal code and nested details.
    #[serde(default)]
    pub code: Option<RegistrationRefusal>,
    /// Suggested retry delay.
    #[serde(default)]
    pub retry_after_ms: Option<u64>,
    /// Capacity scope and absolute deadline.
    #[serde(default)]
    pub capacity: Option<NetworkingCapacity>,
    /// Node binding identity associated with this explanation.
    #[serde(default)]
    pub node_binding: Option<serde_json::Value>,
}

impl NetworkingReason {
    /// Whether this reason requires user action rather than reconnecting.
    #[must_use]
    pub fn requires_action(&self) -> bool {
        use NetworkingReasonKind as Kind;
        use RegistrationRefusal as Code;
        match self.kind {
            Kind::NodeBusy | Kind::NodeUnreachable => false,
            Kind::NoNodeSelected | Kind::NodeCustodyUnavailable | Kind::CustodyExpired => true,
            Kind::RegistrationRefused => !matches!(
                self.code,
                Some(
                    Code::Busy
                        | Code::Capacity
                        | Code::PersistenceUnavailable
                        | Code::SessionRequired
                        | Code::SessionExpired
                )
            ),
        }
    }
}
