// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Every intent kind.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ContactAction, GovernanceOp, SecretInput};
use crate::card_event::CardSlotOutcome;
use crate::ids::{
    AccountId, BackupJobId, CallId, ContextId, DeviceId, GrantId, MomentId, NodeId, ObjectId,
    ParticipantId, StandingApprovalId, SurfaceId, TaskId,
};
use crate::model::{
    AgentAccess, AgentCreateRequest, AgentJobAction, AgentJobKind, AgentPersona,
    AgentRuntimeAction, CardSeen, ConflictResolution, DeleteScope, PinSource, RequestAnswer,
    RequestPolicy,
};
use crate::surface_hash::SurfaceHash;

/// Direction of an own-device sync transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncDirection {
    /// Write a sync bundle for another device.
    Export,
}

/// Whether the app is in the foreground, for the app lock's timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppLockPhase {
    /// The window lost focus or was hidden: the lock timer runs.
    Background,
    /// The window is in front again: the timer stops.
    Foreground,
}

/// A host event a card node takes (`card.session.event`). Its value is a
/// user's text or choice: `Debug` never prints it.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CardInput {
    /// A typed value for a field (text, check, choice, slider, date).
    Input {
        /// The value, in the shape the card's component takes.
        value: serde_json::Value,
    },
    /// A tap on a Button.
    Activate,
    /// A tab selection by index.
    SelectTab {
        /// The tab's index.
        index: u64,
    },
    /// The host opened a modal of the card.
    ModalOpen,
    /// The host closed a modal of the card.
    ModalClose,
}

impl std::fmt::Debug for CardInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Input { .. } => f.debug_struct("Input").field("value", &"<redacted>").finish(),
            Self::Activate => f.write_str("Activate"),
            Self::SelectTab { index } => f.debug_struct("SelectTab").field("index", index).finish(),
            Self::ModalOpen => f.write_str("ModalOpen"),
            Self::ModalClose => f.write_str("ModalClose"),
        }
    }
}

/// Every intent kind.
///
/// Kinds that carry a [`SecretInput`] (unlock, restore,
/// backup export, full backup and restore verification, passphrase
/// rotation, username claim, app lock code answer) are excluded from serde: serializing them is an
/// error and they cannot be deserialized.
#[derive(Debug, Serialize, Deserialize)]
pub enum IntentKind {
    // Conversation.
    /// Retry a send that expired before acceptance.
    RetryMessage {
        /// Failed message.
        moment_id: MomentId,
    },
    /// Send a text message.
    SendMessage {
        /// Target context.
        context_id: ContextId,
        /// Text.
        text: String,
    },
    /// Reply to a message.
    ReplyMessage {
        /// The message replied to.
        moment_id: MomentId,
        /// Text.
        text: String,
    },
    /// Replace the text of one of this account's messages.
    EditMessage {
        /// The edited message.
        moment_id: MomentId,
        /// New text.
        text: String,
    },
    /// Delete a message, on this device only or for everyone (needs a
    /// confirmation). Everyone is limited to this account's own message.
    DeleteMessage {
        /// The message.
        moment_id: MomentId,
        /// Where the message is erased.
        scope: DeleteScope,
    },
    /// Leave a group (needs a confirmation).
    LeaveGroup {
        /// The group conversation.
        context_id: ContextId,
    },
    /// Accept or decline a group invitation.
    AnswerGroupInvitation {
        /// The group.
        group_id: String,
        /// Accept (true) or decline.
        accept: bool,
    },
    /// Add a contact by username.
    AddContactByUsername {
        /// The name, without a leading `@`.
        name: String,
    },
    /// Look a username up in the directory, before adding it.
    ResolveUsername {
        /// The name, with or without a leading `@`.
        name: String,
    },
    /// Re-read this account's username status from the directory, to settle
    /// a claim that was still publishing. Read only.
    UsernameStatusRefresh {
        /// The operation of the claim still publishing, which the directory
        /// answers for; `None` reads the current state.
        operation_id: Option<String>,
    },
    /// Claim or change this account's username. The recovery phrase
    /// authorizes it and is never stored or serialized. Not serializable.
    #[serde(skip)]
    ClaimUsername {
        /// Account.
        account_id: AccountId,
        /// The name, with or without a leading `@`.
        name: String,
        /// The recovery phrase.
        secret: SecretInput,
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
    /// Open the card runtime's session for a card on screen.
    CardOpen {
        /// Moment holding the surface.
        moment_id: MomentId,
        /// Surface (the daemon's `surface_ref`).
        surface_id: SurfaceId,
        /// The app locale (BCP 47).
        locale: String,
        /// The app is in its dark theme.
        dark: bool,
    },
    /// Present an open card in another locale or theme.
    CardPresent {
        /// Moment holding the surface.
        moment_id: MomentId,
        /// Surface.
        surface_id: SurfaceId,
        /// The app locale (BCP 47).
        locale: String,
        /// The app is in its dark theme.
        dark: bool,
    },
    /// A host event on one node of an open card.
    CardInteract {
        /// Moment holding the surface.
        moment_id: MomentId,
        /// Surface.
        surface_id: SurfaceId,
        /// Key of the node the event addresses.
        key: String,
        /// The event.
        event: CardInput,
        /// What the user saw of the node when the event was taken.
        seen: CardSeen,
    },
    /// The host's answer to a slot of a card (link sheet or approval).
    CardSlotAnswer {
        /// Moment holding the surface.
        moment_id: MomentId,
        /// Surface.
        surface_id: SurfaceId,
        /// Slot id, as the card asked for it.
        slot_id: String,
        /// The answer.
        outcome: CardSlotOutcome,
        /// The tree state the user saw when answering.
        seen: CardSeen,
    },
    /// Send the submit a button press asked for.
    CardSubmit {
        /// Moment holding the surface.
        moment_id: MomentId,
        /// Surface.
        surface_id: SurfaceId,
    },
    /// Read a card again: its tree no longer follows the runtime's session.
    CardRefresh {
        /// Moment holding the surface.
        moment_id: MomentId,
        /// Surface.
        surface_id: SurfaceId,
    },
    /// Suspend one card's session (the pool asked for it).
    CardSuspend {
        /// Moment holding the surface.
        moment_id: MomentId,
        /// Surface.
        surface_id: SurfaceId,
    },
    /// Release one card's session (the pool asked for it).
    CardDispose {
        /// Moment holding the surface.
        moment_id: MomentId,
        /// Surface.
        surface_id: SurfaceId,
    },
    /// Suspend every resumed card (the window is hidden or the app locked).
    CardSuspendAll,
    /// Resume the most recently used cards (the window is shown again).
    CardResumeAll,
    /// Keep sessions only for the cards in `surface_ids` (the loaded window).
    CardRetainVisible {
        /// The surfaces still on screen.
        surface_ids: std::collections::BTreeSet<SurfaceId>,
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
    /// Reveal masked content (needs a confirmation; audited).
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
    /// Forget the daemon's entries of closed contact requests, so that it
    /// can take new ones again. Human only; needs the confirmation.
    ClearRequestFloors,
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
    /// Cancel a transfer on its way.
    CancelTransfer {
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
    /// Revoke an approved, unused grant (needs a confirmation).
    RevokeGrant {
        /// Grant.
        grant_id: GrantId,
    },
    /// Revoke a standing approval (needs a confirmation).
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
    /// trust-on-first-use pin needs a confirmation. Human only.
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
        /// Record files that cannot be fetched as unavailable instead of
        /// failing. `false` unless the user explicitly confirmed it after a
        /// backup failed on them.
        allow_missing_attachments: bool,
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
    /// Begin creating an account with a recovery phrase (born-v3). Only
    /// when the daemon offers it ([`crate::Capabilities::born_v3_creation`]).
    BeginCeremony {
        /// The name of the new account. A person's account only: an AI
        /// identity needs a custodian the desktop does not offer yet.
        label: String,
    },
    /// Ask for the recovery phrase, once. The source hands it to the
    /// screen through [`crate::Source::take_recovery_phrase`].
    ShowRecoveryPhrase,
    /// The screen erased the phrase: ask which words to type back. Sent
    /// only after the erasure.
    HideRecoveryPhrase,
    /// Type three words back and set the account passphrase; the one
    /// attempt, sent once as `{words, passphrase}`. Not serializable.
    #[serde(skip)]
    ConfirmCeremony {
        /// The three words, resolved to list words, in the order asked.
        words: Vec<SecretInput>,
        /// The passphrase that unlocks the account on this device.
        passphrase: SecretInput,
    },
    /// Cancel the running ceremony; the daemon erases the phrase.
    CancelCeremony,
    /// Create a named agent. The daemon asks the account's phone to
    /// approve it; the desktop then polls the request. Human only, on an
    /// unlocked, connected person's account.
    AgentCreate {
        /// The owner's account (unlocked, connected, the person's own).
        owner_id: AccountId,
        /// The persona, the runtime choice and the access.
        request: AgentCreateRequest,
    },
    /// Read the custody status of a creation request this desktop started
    /// (after a restart it is read again from its stored id).
    AgentOperationRefresh {
        /// The request id the daemon gave.
        request_id: String,
    },
    /// Change the persona of an agent (name, avatar preset, responsibility).
    AgentPersonaEdit {
        /// The agent's anchor.
        agent: String,
        /// The new persona.
        persona: AgentPersona,
    },
    /// Change who may reach an agent. Sent with the record's base; a stale
    /// base is refused and the record is read again.
    AgentAccessSet {
        /// The agent's anchor.
        agent: String,
        /// The new access.
        access: AgentAccess,
    },
    /// Delete an agent in state U (the record is retired, not erased).
    AgentDelete {
        /// The agent's anchor.
        agent: String,
    },
    /// Pause, resume or remove one PRX job of an agent.
    AgentJob {
        /// The agent's anchor.
        agent: String,
        /// The job's kind.
        kind: AgentJobKind,
        /// The job's id.
        id: String,
        /// What to do.
        action: AgentJobAction,
    },
    /// Read the PRX jobs of an agent (the jobs panel opens, and again after
    /// a job action). Read only; the answer replaces the agent's job list.
    AgentJobsRefresh {
        /// The agent's anchor.
        agent: String,
    },
    /// Approve or deny an agent's approval in a conversation.
    AgentApprovalDecide {
        /// The account that owns the agent and answers: the account the
        /// panel is showing. Refused when it is not the owner of the agent
        /// the approval names.
        owner_id: AccountId,
        /// The approval id.
        approval_id: String,
        /// Approve (true) or deny (false).
        approved: bool,
    },
    /// Run a PRX runtime action on this computer, in process (not on the
    /// bus). The provider key of an install is one-shot and never
    /// serializes.
    AgentRuntime {
        /// The action.
        action: AgentRuntimeAction,
        /// The provider the runtime uses (install only).
        provider: String,
        /// The model the runtime uses (install only).
        model: String,
        /// The provider key (install only).
        #[serde(skip)]
        key: Option<SecretInput>,
        /// The number of this submission (see `AgentHostStatus::nonce`).
        #[serde(default)]
        nonce: u64,
    },
    /// Read the changes to the account's agents since the roster's journal
    /// cursor (`agent.watch`). Read only.
    AgentWatchRefresh,
    /// Begin recovering an account that lives on this device, from its 24
    /// words. Only the user starts it, from the account itself; nothing a
    /// network event carries may. The daemon answers with the account's
    /// public fingerprint.
    BeginRecovery {
        /// The account to recover.
        account_id: AccountId,
    },
    /// Send the words and the account passphrase, once, after the user
    /// confirmed the fingerprint. Not serializable.
    #[serde(skip)]
    ConfirmRecovery {
        /// The fingerprint the user confirmed (the policy checks it is the
        /// one the daemon named).
        anchor: String,
        /// The 24 words, as list words joined by single spaces.
        phrase: SecretInput,
        /// The passphrase of the account on this device.
        passphrase: SecretInput,
    },
    /// Cancel the running recovery.
    CancelRecovery,
    /// Revoke one of the account's devices (needs a confirmation).
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
    /// Answer the account code the daemon asked for an app lock change. An
    /// empty secret dismisses the sheet. Human only.
    #[serde(skip)]
    AnswerAppLockCode {
        /// Account.
        account_id: AccountId,
        /// The account code (empty to dismiss).
        secret: SecretInput,
    },
    /// Turn an account's app lock on or off, and set its timeout. Human only.
    ConfigureAppLock {
        /// Account (must be open).
        account_id: AccountId,
        /// Whether the lock is on.
        enabled: bool,
        /// Background time before the lock, in milliseconds (0 is immediately).
        timeout_ms: u64,
    },
    /// Report that the window went to the background or came to the front.
    AppLockLifecycle {
        /// Account.
        account_id: AccountId,
        /// New phase of the window.
        phase: AppLockPhase,
    },
}
