// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Moments: the inline items of a context timeline, their lifecycle, and
//! every payload kind.

use serde::{Deserialize, Serialize};

use super::conversation::{PeerKind, Role};
use super::grant::Risk;
use super::protected::Protected;
use super::surface::{Block, SurfaceAction};
use crate::ids::{
    CallId, ContextId, GrantId, MomentId, ObjectId, ParticipantId, SurfaceId, TaskId, Timestamp,
};

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
    /// Attached object, shown as an attachment row.
    #[serde(default)]
    pub attachment: Option<ObjectId>,
    /// Franking commitment, shown read-only in the evidence dock.
    #[serde(default)]
    pub franking: Option<String>,
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
    /// Member banned.
    Banned,
    /// Member role changed.
    RoleChanged,
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

/// What a member did to a task, as one of its signed task events says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskEventKind {
    /// Accepted the task (the assignee a started task, or the requester a
    /// proposal).
    Accepted,
    /// Declined it.
    Declined,
    /// Cancelled it (its requester).
    Cancelled,
    /// Let a step that asked go on (approved its approval, or answered).
    Approved {
        /// The step decided, when the event names it.
        step_id: Option<crate::ids::StepId>,
    },
    /// Refused what a step asked.
    Denied {
        /// The step decided, when the event names it.
        step_id: Option<crate::ids::StepId>,
    },
}

/// Payload of a [`MomentKind::TaskEvent`]: a compact line in the timeline
/// for one signed event of a task (its card shows the task itself). The
/// moment's origin is the member who signed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskEventPayload {
    /// The task.
    pub task_id: TaskId,
    /// What happened.
    pub event: TaskEventKind,
    /// Whether a root-signed profile and a current device of the signer
    /// prove its kind for the event's role (the source's
    /// `actor_kind_verified`); false until reported.
    #[serde(default)]
    pub actor_kind_verified: bool,
    /// The signer's authenticated kind (the source's `actor_kind`), when the
    /// source gave one. A kind with `actor_kind_verified` false means the
    /// authenticated kind does not fit the role of the action (for example a
    /// headless AI as the requester of a human-only action); `None` is
    /// unknown (a legacy event), never inferred from the role.
    #[serde(default)]
    pub actor_kind: Option<PeerKind>,
}

/// Payload of a [`MomentKind::NameChange`]: a participant's public name
/// changed through its root-signed profile (an authenticated rename, as the
/// source announced it). The moment's origin is the participant. Shown
/// with the principal id, so a new name cannot pass for someone else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameChangePayload {
    /// The renamed participant.
    pub participant_id: ParticipantId,
    /// The name before.
    pub old_name: String,
    /// The name now.
    pub new_name: String,
}

/// Payload of a [`MomentKind::GroupRename`]: the group's signed name changed
/// (the creator's, or an admin's request the creator applied), as the daemon
/// lists it after a sync. The moment has no author: the signed state names
/// the group's writer, not who asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupRenamePayload {
    /// The name before.
    pub old_name: String,
    /// The name now.
    pub new_name: String,
}

/// Payload of a [`MomentKind::Call`]: a voice call's record in its
/// conversation. The call (state, times, outcome) lives in the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallPayload {
    /// The call.
    pub call_id: CallId,
}

/// One side of a version conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictSide {
    /// Version or hash label.
    pub version: String,
    /// Human summary of this side.
    pub summary: String,
    /// The conflicting lines of this side, for the side-by-side compare.
    #[serde(default)]
    pub lines: Vec<String>,
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
    /// Revealed content; absent while masked. Never printed by `Debug`.
    #[serde(default)]
    pub content: Option<Protected>,
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
    /// One event of a task, as a compact line.
    TaskEvent(TaskEventPayload),
    /// An authenticated public rename of a participant.
    NameChange(NameChangePayload),
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
    /// A voice call's record (derived by the store from the call).
    Call(CallPayload),
    /// The group's signed name changed.
    GroupRename(GroupRenamePayload),
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
