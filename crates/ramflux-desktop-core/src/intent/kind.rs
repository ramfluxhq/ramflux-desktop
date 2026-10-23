// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Every intent kind.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{GovernanceOp, SecretInput};
use crate::conformance::SurfaceHash;
use crate::ids::{
    AccountId, ContextId, DeviceId, GrantId, MomentId, NodeId, ObjectId, ParticipantId,
    StandingApprovalId, SurfaceId, TaskId,
};
use crate::model::{AccountKind, ConflictResolution, PinSource};

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
/// backup export, passphrase rotation) are excluded from serde: serializing them is an error
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
    /// Start a task from a one-sentence request.
    StartTask {
        /// Context.
        context_id: ContextId,
        /// The request text.
        request: String,
    },
    /// Cancel a running task.
    CancelTask {
        /// Task.
        task_id: TaskId,
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
