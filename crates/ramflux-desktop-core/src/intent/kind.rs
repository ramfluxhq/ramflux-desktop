// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Every intent kind.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ContactAction, GovernanceOp, SecretInput};
use crate::conformance::SurfaceHash;
use crate::ids::{
    AccountId, BackupJobId, CallId, ContextId, DeviceId, GrantId, MomentId, NodeId, ObjectId,
    ParticipantId, StandingApprovalId, SurfaceId, TaskId,
};
use crate::model::{AccountKind, ConflictResolution, PinSource, RequestAnswer, RequestPolicy};

/// Direction of an own-device sync transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncDirection {
    /// Write a sync bundle for another device.
    Export,
    /// Read a sync bundle from another device.
    Import,
}

/// Every intent kind.
///
/// Kinds that carry a [`SecretInput`] (unlock, create, activate, restore,
/// backup export, full backup and restore verification, passphrase rotation) are excluded from serde: serializing them is an error
/// and they cannot be deserialized.
#[derive(Debug, Serialize, Deserialize)]
pub enum IntentKind {
    // Conversation.
    /// Send a text message.
    SendMessage {
        /// Target context.
        context_id: ContextId,
        /// Text.
        text: String,
    },
    /// Start a direct conversation with a contact: the first message
    /// creates it. The conversation id is chosen fresh by the caller (never
    /// derived from either identity), so a retry names the same one.
    StartConversation {
        /// The new conversation's id.
        context_id: ContextId,
        /// The contact.
        participant_id: ParticipantId,
        /// The first message.
        text: String,
    },
    /// Send a local file as an object.
    AttachObject {
        /// Target context.
        context_id: ContextId,
        /// Local path chosen by the user.
        path: String,
    },
    /// Pin a context to the stack.
    PinContext {
        /// Context.
        context_id: ContextId,
    },
    /// Unpin a context.
    UnpinContext {
        /// Context.
        context_id: ContextId,
    },
    /// Mute or unmute a context.
    MuteContext {
        /// Context.
        context_id: ContextId,
        /// New state.
        muted: bool,
    },
    /// Set or clear the disappearing-message timer.
    SetDisappearing {
        /// Context.
        context_id: ContextId,
        /// Timer in seconds; `None` turns it off.
        seconds: Option<u32>,
    },
    /// Ask an AI participant of the conversation to do something
    /// (`task.start`).
    StartTask {
        /// Conversation.
        context_id: ContextId,
        /// The participant asked to do it; an agent member of the conversation.
        assignee: ParticipantId,
        /// What to do, in one sentence.
        text: String,
    },
    /// Confirm a task the assignee proposed: the requester starts it.
    /// Human only.
    AcceptTaskProposal {
        /// The proposed task.
        task_id: TaskId,
    },
    /// Cancel a task, or decline a proposal. Only its requester may.
    CancelTask {
        /// Task.
        task_id: TaskId,
    },
    /// Send again the decision on a task step that could not be delivered
    /// to the assignee: the same signed decision, unchanged. Only its
    /// requester may.
    RetryTaskDecision {
        /// Task.
        task_id: TaskId,
    },

    // Calls.
    /// Place a voice call to the other participant of a direct
    /// conversation. Human only.
    StartCall {
        /// The direct conversation.
        context_id: ContextId,
    },
    /// Answer a call ringing on this device. Human only.
    AnswerCall {
        /// The call.
        call_id: CallId,
    },
    /// Decline a call ringing on this device. Human only.
    RejectCall {
        /// The call.
        call_id: CallId,
    },
    /// Hang up a call (or cancel one still ringing). Human only.
    HangUpCall {
        /// The call.
        call_id: CallId,
    },

    // Moments.
    /// Invoke an action of an agent surface.
    SurfaceAction {
        /// Moment holding the surface.
        moment_id: MomentId,
        /// Surface.
        surface_id: SurfaceId,
        /// Action id.
        action_id: String,
        /// Form values collected by the surface.
        #[serde(default)]
        values: Value,
        /// The surface version the action was shown on. Policy refuses the
        /// intent when the surface changed since.
        surface_hash: SurfaceHash,
        /// The version number the action was shown on (with the hash).
        #[serde(default)]
        version: u64,
    },
    /// Ask the author of an out-of-sync surface for its current state.
    RequestSurfaceUpdate {
        /// Moment holding the surface.
        moment_id: MomentId,
        /// Surface.
        surface_id: SurfaceId,
    },
    /// Approve a pending grant.
    Approve {
        /// Grant.
        grant_id: GrantId,
    },
    /// Deny a pending grant.
    Deny {
        /// Grant.
        grant_id: GrantId,
    },
    /// Reveal masked content (needs a hold; audited).
    Reveal {
        /// PrivacyReveal moment.
        moment_id: MomentId,
    },
    /// Resolve a CAS conflict.
    ResolveConflict {
        /// Conflict moment.
        moment_id: MomentId,
        /// Chosen resolution.
        resolution: ConflictResolution,
    },
    /// Accept a contact request.
    AcceptContact {
        /// ContactRequest moment.
        moment_id: MomentId,
    },
    /// Reject a contact request.
    RejectContact {
        /// ContactRequest moment.
        moment_id: MomentId,
    },
    /// Mark a participant's safety number as verified out of band. Human only.
    VerifySafetyNumber {
        /// Participant.
        participant_id: ParticipantId,
    },
    /// Continue the offered context on this device. Human only.
    AcceptHandoff {
        /// Handoff moment.
        moment_id: MomentId,
    },
    /// Decline a handoff offer. Human only.
    DeclineHandoff {
        /// Handoff moment.
        moment_id: MomentId,
    },

    // Contacts.
    /// Add a contact by the id they shared out of band. Human only.
    AddContact {
        /// The contact's id (their principal id on the daemon).
        participant_id: ParticipantId,
    },
    /// Add a contact from the contact card they shared (pasted text or
    /// link, or a card file's contents). Human only.
    AddContactCard {
        /// The card, as shared.
        card: String,
        /// The provisional root (from a name lookup) the user confirmed
        /// the card's root replaces, after comparing both; `None` for a
        /// plain add.
        #[serde(default)]
        replace_provisional_root: Option<String>,
    },
    /// Export this account's own contact card to share it.
    ExportContactCard,
    /// Answer an incoming contact request. Human only.
    AnswerContactRequest {
        /// The daemon's request id.
        request_id: String,
        /// The answer.
        answer: RequestAnswer,
    },
    /// Set how contact requests are answered. Human only.
    SetContactRequestPolicy {
        /// The setting.
        policy: RequestPolicy,
    },
    /// Act on the contact link with a participant.
    Contact {
        /// Participant.
        participant_id: ParticipantId,
        /// Action.
        action: ContactAction,
    },

    // Objects.
    /// Pull (download) an inbound object.
    PullObject {
        /// Object.
        object_id: ObjectId,
    },
    /// Save a downloaded object to a local path.
    SaveObject {
        /// Object.
        object_id: ObjectId,
        /// Local path chosen by the user.
        path: String,
    },
    /// Retry a failed transfer.
    RetryTransfer {
        /// Object.
        object_id: ObjectId,
    },

    // Governance and approvals.
    /// Create a group: its conversation (under a fresh id chosen by the
    /// caller, so a retry names the same one), its name, and the accepted
    /// contacts added as its first members. Human only.
    CreateGroup {
        /// The new group conversation's id.
        context_id: ContextId,
        /// The group's name.
        title: String,
        /// Contacts to add as members.
        members: Vec<ParticipantId>,
    },
    /// Rename a group conversation.
    RenameContext {
        /// Group conversation.
        context_id: ContextId,
        /// New name.
        title: String,
    },
    /// Change group or space membership or roles.
    Governance {
        /// Group or space.
        context_id: ContextId,
        /// Operation.
        op: GovernanceOp,
    },
    /// Revoke an approved, unused grant (needs a hold).
    RevokeGrant {
        /// Grant.
        grant_id: GrantId,
    },
    /// Revoke a standing approval (needs a hold).
    RevokeStandingApproval {
        /// Standing approval.
        approval_id: StandingApprovalId,
    },

    // Entry: accounts and nodes.
    /// Unlock an account. Not serializable.
    #[serde(skip)]
    Unlock {
        /// Account.
        account_id: AccountId,
        /// One-shot secret.
        secret: SecretInput,
    },
    /// Create an account bound to a confirmed node. Not serializable.
    #[serde(skip)]
    CreateAccount {
        /// Interactive or headless.
        kind: AccountKind,
        /// Display label.
        label: String,
        /// Home node.
        node_id: NodeId,
        /// New passphrase.
        secret: SecretInput,
    },
    /// Join an existing identity as a new device. Not serializable.
    #[serde(skip)]
    ActivateDevice {
        /// Home node of the identity.
        node_id: NodeId,
        /// Label of this device.
        device_label: String,
        /// Activation code shown on an existing device.
        code: SecretInput,
    },
    /// Restore an account from an encrypted backup. Not serializable.
    #[serde(skip)]
    RestoreBackup {
        /// Backup file chosen by the user.
        path: String,
        /// Backup passphrase.
        secret: SecretInput,
    },
    /// Discover a node from a URL; the source answers with an unconfirmed node.
    AddNode {
        /// Node URL or id.
        url: String,
    },
    /// Confirm and pin a discovered node. Audited like an approval; a
    /// trust-on-first-use pin needs a hold. Human only.
    ConfirmNode {
        /// Discovered node.
        node_id: NodeId,
        /// Pin source the user is accepting; must match the discovery.
        pin_source: PinSource,
    },

    // Settings.
    /// Lock an account.
    LockAccount {
        /// Account.
        account_id: AccountId,
    },
    /// Rotate an account passphrase. Not serializable.
    #[serde(skip)]
    RotatePassphrase {
        /// Account.
        account_id: AccountId,
        /// Current passphrase.
        current: SecretInput,
        /// New passphrase.
        next: SecretInput,
    },
    /// Export an encrypted backup to a local path. Not serializable.
    #[serde(skip)]
    ExportBackup {
        /// Account.
        account_id: AccountId,
        /// Local path chosen by the user.
        path: String,
        /// Passphrase the backup is encrypted with.
        secret: SecretInput,
    },
    /// Write a full backup of an account (everything it holds, every file)
    /// to a local path. Not serializable.
    #[serde(skip)]
    CreateBackup {
        /// Account.
        account_id: AccountId,
        /// Local path chosen by the user.
        path: String,
        /// Passphrase the backup is encrypted with.
        secret: SecretInput,
    },
    /// Read and verify a whole backup file into staging, without
    /// installing anything yet. Not serializable.
    #[serde(skip)]
    VerifyRestore {
        /// Backup file chosen by the user.
        path: String,
        /// Backup passphrase.
        secret: SecretInput,
    },
    /// Install a verified backup the user reviewed: added as a new account,
    /// or merged into the account already here (the newer item wins).
    InstallRestore {
        /// The verified restore job.
        job_id: BackupJobId,
    },
    /// Cancel a running backup, or a restore before it installs (its
    /// staged copy is discarded).
    CancelBackupJob {
        /// Job.
        job_id: BackupJobId,
    },
    /// Revoke one of the account's devices (needs a hold).
    RevokeDevice {
        /// Device.
        device_id: DeviceId,
    },
    /// Export or import own-device sync data.
    SyncDevices {
        /// Direction.
        direction: SyncDirection,
        /// Local path chosen by the user.
        path: String,
    },
    /// Probe a node's gateway and relay.
    ProbeNode {
        /// Node.
        node_id: NodeId,
    },
    /// Remove a node profile; refused while an account is bound to it.
    RemoveNode {
        /// Node.
        node_id: NodeId,
    },
    /// Make a node the default for new accounts.
    SetDefaultNode {
        /// Node.
        node_id: NodeId,
    },
    /// Enable or disable a node for new accounts.
    SetNodeEnabled {
        /// Node.
        node_id: NodeId,
        /// New state.
        enabled: bool,
    },
    /// Export a redacted diagnostics bundle.
    ExportDiagnostics {
        /// Local path chosen by the user.
        path: String,
    },
}
