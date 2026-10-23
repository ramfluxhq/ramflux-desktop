// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Normalized domain model.
//!
//! The store keeps every entity keyed by its id. Moments reference their
//! context; surfaces, grants, objects and tasks are referenced by id from the
//! moment payloads that present them.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{
    AccountId, BlockId, ContextId, GrantId, MomentId, NodeId, ObjectId, ParticipantId, SurfaceId,
    TaskId, Timestamp,
};

/// Who originates an intent or a task. Humans and agents share one pipeline (R6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Actor {
    /// The attended human user of this desktop.
    Human,
    /// An agent participant acting through the same policy as the human.
    Agent(ParticipantId),
}

/// Whether the local account is unlocked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LockState {
    /// Secrets are sealed; the user must unlock first.
    #[default]
    Locked,
    /// The account is usable.
    Unlocked,
}

/// A local account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// Account id.
    pub id: AccountId,
    /// Human-readable label.
    pub label: String,
    /// Home node, once joined.
    #[serde(default)]
    pub node_id: Option<NodeId>,
    /// This device's id, once registered.
    #[serde(default)]
    pub device_id: Option<String>,
    /// Lock state.
    #[serde(default)]
    pub lock_state: LockState,
}

/// How a node endpoint was pinned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PinSource {
    /// Shipped bootstrap list.
    Bootstrap,
    /// From an invite.
    Invite,
    /// Entered by the user.
    Manual,
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

/// A Ramflux node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    /// Node id.
    pub id: NodeId,
    /// Known endpoints.
    #[serde(default)]
    pub endpoints: Vec<String>,
    /// Where the pin came from.
    pub pin_source: PinSource,
    /// Last observed health.
    #[serde(default)]
    pub health: NodeHealth,
}

/// What kind of party a participant is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParticipantKind {
    /// A person.
    Human,
    /// An AI agent.
    Agent,
    /// One of the user's own devices.
    Device,
    /// A node acting as a participant.
    Node,
}

/// Key verification state for a participant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SafetyState {
    /// Keys not verified out of band.
    #[default]
    Unverified,
    /// Safety number verified.
    Verified,
    /// Identity key changed since last seen.
    KeyChanged,
    /// Identity fork detected.
    Forked,
}

/// A participant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Participant {
    /// Participant id.
    pub id: ParticipantId,
    /// Kind.
    pub kind: ParticipantKind,
    /// Display name.
    pub display: String,
    /// Key verification state.
    #[serde(default)]
    pub safety: SafetyState,
    /// Whether the participant is in the user's contact list.
    #[serde(default)]
    pub contact: bool,
}

/// Context shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextKind {
    /// One-to-one.
    Direct,
    /// Group conversation.
    Group,
    /// Long-lived shared space.
    Space,
}

/// A conversation context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    /// Context id.
    pub id: ContextId,
    /// Shape.
    pub kind: ContextKind,
    /// Display title.
    pub title: String,
    /// Members in display order.
    #[serde(default)]
    pub participants: Vec<ParticipantId>,
    /// Pinned to the context stack.
    #[serde(default)]
    pub pinned: bool,
    /// Unread moment count reported by the source.
    #[serde(default)]
    pub unread: u32,
}

/// Moment lifecycle. "Needs me" and "in flight" views derive from it.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum Lifecycle {
    /// Created, not started (also used for optimistic local moments, R5).
    #[default]
    Pending,
    /// Work is running.
    InProgress,
    /// Waiting on the user.
    NeedsMe,
    /// Finished successfully. Terminal.
    Done,
    /// Finished with failure. Terminal.
    Failed,
    /// Dismissed by the user. Terminal.
    Dismissed,
}

impl Lifecycle {
    /// Returns true for `Done`, `Failed` and `Dismissed`.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Dismissed)
    }

    /// Returns whether `from -> to` is a legal transition.
    ///
    /// Self-transitions are not transitions and return false.
    pub const fn can_transition(from: Self, to: Self) -> bool {
        use Lifecycle::{Dismissed, Done, Failed, InProgress, NeedsMe, Pending};
        matches!(
            (from, to),
            (Pending, InProgress | NeedsMe | Done | Failed)
                | (InProgress, NeedsMe | Done | Failed)
                | (NeedsMe, InProgress | Done | Failed | Dismissed)
        )
    }
}

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

/// Catalog of block kinds a surface may contain.
///
/// Unknown kinds deserialize to [`BlockKind::Fallback`] and never fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BlockKind {
    /// Plain text.
    Text,
    /// Markdown document (rendered without scripts).
    Markdown,
    /// Heading.
    Heading,
    /// Key/value pairs.
    KeyValue,
    /// Single metric.
    Metric,
    /// List of items.
    List,
    /// Table rendered in fallback form.
    TableFallback,
    /// Form container.
    Form,
    /// Text input.
    Input,
    /// Selection input.
    Select,
    /// Progress indicator.
    Progress,
    /// Streaming log lines.
    LogStream,
    /// Warning callout.
    Warning,
    /// Error callout.
    Error,
    /// Success callout.
    Success,
    /// Approval request bound to a grant.
    Approval,
    /// Group of actions.
    ActionGroup,
    /// File preview.
    FilePreview,
    /// Fallback for unsupported or unknown blocks.
    #[serde(other)]
    Fallback,
}

/// One block of an agent surface. `data` is kind-specific and untrusted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    /// Block id, unique within its surface.
    pub id: BlockId,
    /// Kind.
    pub kind: BlockKind,
    /// Kind-specific data.
    #[serde(default)]
    pub data: Value,
}

/// An action offered by a surface. Invoking it becomes an intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceAction {
    /// Action id, unique within its surface.
    pub id: String,
    /// Button label.
    pub label: String,
    /// Declared risk.
    #[serde(default)]
    pub risk: Risk,
    /// When set, the action is shown disabled with this reason.
    #[serde(default)]
    pub disabled_reason: Option<String>,
}

/// Payload of a [`MomentKind::Message`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessagePayload {
    /// Body text.
    pub text: String,
    /// Delivery receipt.
    #[serde(default)]
    pub receipt: Receipt,
    /// The message was deleted; render a tombstone.
    #[serde(default)]
    pub tombstone: bool,
}

/// Delivery receipt of an outbound message.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Receipt {
    /// Not yet accepted by the node.
    Sending,
    /// Accepted by the node.
    #[default]
    Sent,
    /// Delivered to the recipient device.
    Delivered,
    /// Read by the recipient.
    Read,
}

/// Payload of a [`MomentKind::AgentSurface`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSurfacePayload {
    /// Surface id.
    pub surface_id: SurfaceId,
    /// Text shown when the surface cannot be rendered.
    #[serde(default)]
    pub fallback_text: String,
    /// Ordered blocks.
    #[serde(default)]
    pub blocks: Vec<Block>,
    /// Offered actions.
    #[serde(default)]
    pub actions: Vec<SurfaceAction>,
    /// More patches are expected.
    #[serde(default)]
    pub streaming: bool,
}

/// Payload of a [`MomentKind::ObjectTransfer`]. Transfer state lives on the object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectTransferPayload {
    /// Transferred object.
    pub object_id: ObjectId,
}

/// Payload of a [`MomentKind::Intervention`]: an approval bound to a grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterventionPayload {
    /// The grant the decision applies to.
    pub grant_id: GrantId,
    /// Effective risk shown to the user.
    pub risk: Risk,
    /// What is being requested, in plain words.
    pub summary: String,
}

/// What changed about a participant's keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SafetyChangeKind {
    /// Identity key changed.
    KeyChanged,
    /// Identity fork detected.
    Forked,
    /// Safety number verified.
    Verified,
}

/// Payload of a [`MomentKind::SafetyChange`]. Sticky until resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafetyChangePayload {
    /// Affected participant.
    pub participant_id: ParticipantId,
    /// Change.
    pub change: SafetyChangeKind,
}

/// Group governance action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GovernanceAction {
    /// Invitation sent.
    Invited,
    /// Member joined.
    Joined,
    /// Member left.
    Left,
    /// Member removed.
    Removed,
    /// Member role changed.
    RoleChanged,
}

/// Member role in a group or space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// Owner.
    Owner,
    /// Administrator.
    Admin,
    /// Regular member.
    Member,
    /// Guest with limited rights.
    Guest,
}

/// Payload of a [`MomentKind::Governance`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernancePayload {
    /// Action.
    pub action: GovernanceAction,
    /// Affected member.
    pub target: ParticipantId,
    /// New role, for invitations and role changes.
    #[serde(default)]
    pub role: Option<Role>,
}

/// Payload of a [`MomentKind::ContactRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactRequestPayload {
    /// Requesting participant.
    pub from: ParticipantId,
    /// Optional note.
    #[serde(default)]
    pub note: String,
}

/// Payload of a [`MomentKind::TaskProgress`]. Steps live on the task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskProgressPayload {
    /// Presented task.
    pub task_id: TaskId,
}

/// One side of a version conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictSide {
    /// Version or hash label.
    pub version: String,
    /// Human summary of this side.
    pub summary: String,
}

/// How a conflict was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictResolution {
    /// Keep the local side.
    KeepLocal,
    /// Keep the remote side.
    KeepRemote,
    /// Keep both as separate objects.
    KeepBoth,
}

/// Payload of a [`MomentKind::Conflict`] (CAS version mismatch).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictPayload {
    /// Conflicting object.
    pub object_id: ObjectId,
    /// Local side.
    pub local: ConflictSide,
    /// Remote side.
    pub remote: ConflictSide,
    /// Chosen resolution, once resolved.
    #[serde(default)]
    pub resolution: Option<ConflictResolution>,
}

/// Audit record of a privacy reveal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevealRecord {
    /// Who revealed.
    pub by: ParticipantId,
    /// When.
    pub at: Timestamp,
}

/// Payload of a [`MomentKind::PrivacyReveal`]: masked content plus audited reveal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyRevealPayload {
    /// Label shown over the masked content.
    pub label: String,
    /// Audit record once revealed.
    #[serde(default)]
    pub revealed: Option<RevealRecord>,
    /// Revealed content; absent while masked.
    #[serde(default)]
    pub content: Option<String>,
}

/// Payload of a [`MomentKind::Handoff`] (continue on another device).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffPayload {
    /// Device that offers the handoff.
    pub from_device: ParticipantId,
    /// Device that accepted, if any.
    #[serde(default)]
    pub to_device: Option<ParticipantId>,
    /// What is being handed off.
    pub summary: String,
}

/// Payload of a [`MomentKind::Fallback`] (capability downgrade notice).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FallbackPayload {
    /// Why the original content cannot be shown.
    pub reason: String,
    /// Text to show instead.
    pub text: String,
}

/// Severity of a system moment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SystemLevel {
    /// Informational.
    #[default]
    Info,
    /// Warning.
    Warning,
    /// Error.
    Error,
}

/// Payload of a [`MomentKind::System`] (device or node status note).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemPayload {
    /// Severity.
    #[serde(default)]
    pub level: SystemLevel,
    /// Text.
    pub text: String,
}

/// Every inline moment kind. Each variant carries its own payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MomentKind {
    /// Text message.
    Message(MessagePayload),
    /// Declarative agent surface.
    AgentSurface(AgentSurfacePayload),
    /// Object transfer.
    ObjectTransfer(ObjectTransferPayload),
    /// Approval intervention.
    Intervention(InterventionPayload),
    /// Key safety change.
    SafetyChange(SafetyChangePayload),
    /// Group governance.
    Governance(GovernancePayload),
    /// Contact request.
    ContactRequest(ContactRequestPayload),
    /// Multi-step task progress.
    TaskProgress(TaskProgressPayload),
    /// CAS conflict.
    Conflict(ConflictPayload),
    /// Masked content with audited reveal.
    PrivacyReveal(PrivacyRevealPayload),
    /// Cross-device handoff.
    Handoff(HandoffPayload),
    /// Capability downgrade notice.
    Fallback(FallbackPayload),
    /// System status note.
    System(SystemPayload),
}

/// A moment in a context timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Moment {
    /// Moment id.
    pub id: MomentId,
    /// Owning context.
    pub context_id: ContextId,
    /// Originating participant.
    pub origin: ParticipantId,
    /// When it happened.
    pub at: Timestamp,
    /// Kind and payload.
    pub kind: MomentKind,
    /// Lifecycle.
    #[serde(default)]
    pub lifecycle: Lifecycle,
}

/// Object transfer direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    /// Received.
    Inbound,
    /// Sent.
    Outbound,
}

/// Object transfer state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferState {
    /// Waiting to start.
    Queued,
    /// Bytes moving.
    Transferring {
        /// Bytes transferred so far.
        transferred: u64,
    },
    /// Finished.
    Complete,
    /// Failed.
    Failed {
        /// Failure reason.
        reason: String,
    },
}

/// A transferable object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Object {
    /// Object id.
    pub id: ObjectId,
    /// File name.
    pub name: String,
    /// Size in bytes.
    pub size: u64,
    /// Direction.
    pub direction: Direction,
    /// Transfer state.
    pub transfer: TransferState,
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
}

/// One step of a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskStep {
    /// Step label.
    pub label: String,
    /// Step lifecycle.
    #[serde(default)]
    pub lifecycle: Lifecycle,
}

/// A multi-step unit of work started from an intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// Task id.
    pub id: TaskId,
    /// Opaque correlation id of the originating intent.
    #[serde(default)]
    pub intent_id: Option<String>,
    /// Context the task belongs to.
    pub context_id: ContextId,
    /// Who started it.
    pub actor: Actor,
    /// Title.
    pub title: String,
    /// Ordered steps.
    #[serde(default)]
    pub steps: Vec<TaskStep>,
    /// Task lifecycle.
    #[serde(default)]
    pub lifecycle: Lifecycle,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_transition_table() {
        use Lifecycle::*;
        let all = [Pending, InProgress, NeedsMe, Done, Failed, Dismissed];
        let allowed = [
            (Pending, InProgress),
            (Pending, NeedsMe),
            (Pending, Done),
            (Pending, Failed),
            (InProgress, NeedsMe),
            (InProgress, Done),
            (InProgress, Failed),
            (NeedsMe, InProgress),
            (NeedsMe, Done),
            (NeedsMe, Failed),
            (NeedsMe, Dismissed),
        ];
        for from in all {
            for to in all {
                assert_eq!(
                    Lifecycle::can_transition(from, to),
                    allowed.contains(&(from, to)),
                    "{from:?} -> {to:?}"
                );
            }
        }
    }

    #[test]
    fn terminal_states_are_final() {
        for state in [Lifecycle::Done, Lifecycle::Failed, Lifecycle::Dismissed] {
            assert!(state.is_terminal());
        }
        assert!(!Lifecycle::NeedsMe.is_terminal());
    }
}
