// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Why an intent was refused.

use thiserror::Error;

use crate::ids::{
    AccountId, BackupJobId, CallId, ContextId, DeviceId, GrantId, MomentId, NodeId, ObjectId,
    ParticipantId, StandingApprovalId, TaskId,
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
    /// The surface is behind its author; nothing can be acted on until it
    /// catches up.
    #[error("surface of moment {0} is out of sync with its author")]
    SurfaceOutOfSync(MomentId),
    /// Only an out-of-sync surface needs the author's current state.
    #[error("surface of moment {0} is in sync")]
    SurfaceInSync(MomentId),
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
    /// Only the task's requester may do this.
    #[error("only the requester of task {0} may do this")]
    NotRequester(TaskId),
    /// The task is not a proposal waiting for confirmation.
    #[error("task {0} is not a proposal")]
    NotAProposal(TaskId),
    /// No decision of the task waits to be sent again.
    #[error("task {0} has no undelivered decision")]
    NothingToRetry(TaskId),
    /// A task is asked of an AI participant only.
    #[error("{0} is not an AI participant")]
    NotAnAgent(ParticipantId),
    /// The surface was closed; it is a frozen record.
    #[error("surface of moment {0} is closed")]
    SurfaceClosed(MomentId),
    /// Context already in the requested pin state.
    #[error("context {0} is already in that pin state")]
    PinUnchanged(ContextId),
    /// Grant expiry is at or before the intent time.
    #[error("grant {0} has expired")]
    GrantExpired(GrantId),
    /// Participant not in the store.
    #[error("unknown participant {0}")]
    UnknownParticipant(ParticipantId),
    /// No contact request with this id.
    #[error("unknown contact request {0}")]
    UnknownContactRequest(String),
    /// The contact request was answered (or expired) already.
    #[error("contact request {0} is not waiting for an answer")]
    ContactRequestNotPending(String),
    /// No closed contact request entry can be forgotten now.
    #[error("there are no old contact request entries to forget")]
    NothingToForget,
    /// The contact request setting is not known yet; clearing keeps it.
    #[error("the contact request setting is not known yet")]
    ContactRequestSettingUnknown,
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
    /// The typed contact id cannot be used.
    #[error("contact id is not usable: {0:?}")]
    InvalidContactId(crate::intent::ContactIdProblem),
    /// The pasted or imported contact card cannot be used.
    #[error("contact card is not usable: {0:?}")]
    InvalidContactCard(crate::intent::ContactCardProblem),
    /// The contact is listed but this account cannot send to them yet (the
    /// daemon holds none of their verified devices).
    #[error("{0} is not reachable yet")]
    NotReachable(ParticipantId),
    /// The new group's name or member list cannot be used.
    #[error("group is not usable: {0:?}")]
    InvalidGroup(super::GroupProblem),
    /// The contact link is not in a state that allows the intent.
    #[error("contact {participant} is {link:?}")]
    ContactLinkState {
        /// Participant.
        participant: ParticipantId,
        /// Current link.
        link: crate::model::ContactLink,
    },
    /// A new conversation would reuse the id of a known context.
    #[error("context {0} already exists")]
    ContextExists(ContextId),
    /// The participant's key changed or forked and is not verified again;
    /// a new conversation, group or call waits until the user checks the
    /// safety number. A stale verification is never this refusal.
    #[error("the safety number of {0} must be verified again first")]
    SafetyUnresolved(ParticipantId),
    /// A conversation with this account's own identity.
    #[error("{0} is this account")]
    SelfParticipant(ParticipantId),
    /// The device running this client cannot revoke itself.
    #[error("device {0} is this device")]
    ThisDevice(DeviceId),
    /// Call not in the store.
    #[error("unknown call {0}")]
    UnknownCall(CallId),
    /// Voice calls are one-to-one: only in a direct conversation.
    #[error("context {0} is not a direct conversation")]
    CallNeedsDirect(ContextId),
    /// Another call is under way (one call at a time).
    #[error("another call is under way")]
    CallInProgress,
    /// The call is not in a state this intent applies to (answering or
    /// declining a call that no longer rings, hanging up an ended one).
    #[error("call {0} cannot do this in its current state")]
    CallState(CallId),
    /// Backup or restore job not in the store.
    #[error("unknown backup job {0}")]
    UnknownBackupJob(BackupJobId),
    /// Another backup or restore is running (one at a time).
    #[error("another backup or restore is running")]
    BackupInProgress,
    /// The job is not in a phase this intent applies to (installing a
    /// restore that is not verified, cancelling one that is installing or
    /// has ended, installing a backup).
    #[error("backup job {0} cannot do this in its current phase")]
    BackupJobPhase(BackupJobId),
    /// The daemon does not offer account creation with a recovery phrase.
    #[error("account creation with a recovery phrase is not offered")]
    CeremonyNotOffered,
    /// The ceremony is not in a phase this intent applies to (or none runs).
    #[error("the recovery phrase ceremony cannot do this now")]
    CeremonyPhase,
    /// The confirmation needs exactly three words, none empty.
    #[error("confirming needs exactly three words")]
    CeremonyWords,
    /// The account cannot custody an AI participant.
    #[error("this account cannot be the custodian of an AI participant: {0:?}")]
    AgentCustody(crate::model::AgentCustodyProblem),
    /// The credentials of the AI participant cannot be exported now.
    #[error("the AI participant's credentials cannot be exported: {0:?}")]
    AgentExport(crate::model::AgentExportRefusal),
    /// The recovery is not in a phase this intent applies to (or none runs).
    #[error("the recovery cannot do this now")]
    RecoveryPhase,
    /// The fingerprint sent is not the one the daemon named.
    #[error("the fingerprint is not the one of the account being recovered")]
    RecoveryAnchor,
    /// The recovery phrase must be 24 recovery words.
    #[error("a recovery phrase has 24 recovery words")]
    RecoveryWords,
    /// Only a person's account has a recovery phrase.
    #[error("only a person's account can be recovered with a phrase")]
    RecoveryNotForKind,
}
