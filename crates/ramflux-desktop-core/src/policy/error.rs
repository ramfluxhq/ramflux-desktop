// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Why an intent was refused.

use thiserror::Error;

use crate::ids::{
    AccountId, ContextId, DeviceId, GrantId, MomentId, NodeId, ObjectId, ParticipantId,
    StandingApprovalId, TaskId,
};

/// Why an intent was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IntentError {
    /// Acting agent is not a known agent participant.
    #[error("actor {0} is not a known agent")]
    UnknownAgent(ParticipantId),
    /// Agents may not perform this intent.
    #[error("agents may not {0}")]
    AgentForbidden(&'static str),
    /// Context not in the store.
    #[error("unknown context {0}")]
    UnknownContext(ContextId),
    /// Moment not in the store.
    #[error("unknown moment {0}")]
    UnknownMoment(MomentId),
    /// Grant not in the store.
    #[error("unknown grant {0}")]
    UnknownGrant(GrantId),
    /// Task not in the store.
    #[error("unknown task {0}")]
    UnknownTask(TaskId),
    /// Account not in the store.
    #[error("unknown account {0}")]
    UnknownAccount(AccountId),
    /// Object not in the store.
    #[error("unknown object {0}")]
    UnknownObject(ObjectId),
    /// Node not in the store.
    #[error("unknown node {0}")]
    UnknownNode(NodeId),
    /// Device not in the store.
    #[error("unknown device {0}")]
    UnknownDevice(DeviceId),
    /// Standing approval not in the store.
    #[error("unknown standing approval {0}")]
    UnknownStandingApproval(StandingApprovalId),
    /// Text or path is empty.
    #[error("{0} must not be empty")]
    Empty(&'static str),
    /// Grant already decided.
    #[error("grant {0} is not pending")]
    GrantNotPending(GrantId),
    /// Only approved grants can be revoked.
    #[error("grant {0} is not approved")]
    GrantNotApproved(GrantId),
    /// The standing approval is already revoked.
    #[error("standing approval {0} is already revoked")]
    StandingAlreadyRevoked(StandingApprovalId),
    /// The moment is not of the kind the intent targets.
    #[error("moment {moment} is not a {expected} moment")]
    WrongMomentKind {
        /// Moment.
        moment: MomentId,
        /// Expected kind name.
        expected: &'static str,
    },
    /// The moment no longer accepts this intent.
    #[error("moment {0} is not awaiting input")]
    NotActionable(MomentId),
    /// Surface or action not found on the moment.
    #[error("unknown action {action} on moment {moment}")]
    UnknownAction {
        /// Moment.
        moment: MomentId,
        /// Action id.
        action: String,
    },
    /// Action is disabled.
    #[error("action {action} is disabled: {reason}")]
    ActionDisabled {
        /// Action id.
        action: String,
        /// Reason shown to the user.
        reason: String,
    },
    /// The surface changed after the action was shown; the user reviews it again.
    #[error("surface of moment {0} changed since it was shown")]
    SurfaceChanged(MomentId),
    /// A patch left the surface out of date; it waits for a full copy.
    #[error("surface of moment {0} is out of date")]
    SurfaceConflict(MomentId),
    /// The surface exceeds a renderer limit and shows only its fallback.
    #[error("surface of moment {0} exceeds a renderer limit")]
    SurfaceOverLimit(MomentId),
    /// The backend refused the grant; it is closed and never retried.
    #[error("grant {grant} failed: {failure:?}")]
    GrantFailed {
        /// Grant.
        grant: GrantId,
        /// Why.
        failure: crate::model::GrantFailure,
    },
    /// Task already finished.
    #[error("task {0} is already finished")]
    TaskFinished(TaskId),
    /// Context already in the requested pin state.
    #[error("context {0} is already in that pin state")]
    PinUnchanged(ContextId),
    /// Grant expiry is at or before the intent time.
    #[error("grant {0} has expired")]
    GrantExpired(GrantId),
    /// Participant not in the store.
    #[error("unknown participant {0}")]
    UnknownParticipant(ParticipantId),
    /// Safety number already verified.
    #[error("participant {0} is already verified")]
    AlreadyVerified(ParticipantId),
    /// Governance on a direct context.
    #[error("context {0} is not a group or space")]
    NotAGroup(ContextId),
    /// Target is not a member of the context.
    #[error("{participant} is not a member of {context}")]
    NotAMember {
        /// Context.
        context: ContextId,
        /// Participant.
        participant: ParticipantId,
    },
    /// Target is already a member of the context.
    #[error("{participant} is already a member of {context}")]
    AlreadyMember {
        /// Context.
        context: ContextId,
        /// Participant.
        participant: ParticipantId,
    },
    /// The object is not in a state that allows this transfer action.
    #[error("object {object} cannot {action} in its current state")]
    TransferState {
        /// Object.
        object: ObjectId,
        /// Attempted action.
        action: &'static str,
    },
    /// The node is already confirmed and pinned.
    #[error("node {0} is already confirmed")]
    NodeAlreadyConfirmed(NodeId),
    /// The node has not been confirmed yet.
    #[error("node {0} is not confirmed")]
    NodeNotConfirmed(NodeId),
    /// The confirmed pin source differs from the discovery.
    #[error("node {0} was discovered with a different pin source")]
    PinMismatch(NodeId),
    /// An account is bound to the node.
    #[error("node {0} has a bound account")]
    NodeBound(NodeId),
    /// A disabled node cannot be the default, and the default cannot be disabled.
    #[error("node {0} cannot be both default and disabled")]
    DefaultDisabled(NodeId),
    /// The device is already revoked.
    #[error("device {0} is already revoked")]
    DeviceRevoked(DeviceId),
    /// The backend behind the source cannot perform the method.
    #[error("{method} is not available on this backend: {reason}")]
    UnsupportedByBackend {
        /// Contract method.
        method: &'static str,
        /// Reason shown to the user.
        reason: String,
    },
    /// The device running this client cannot revoke itself.
    #[error("device {0} is this device")]
    ThisDevice(DeviceId),
}
