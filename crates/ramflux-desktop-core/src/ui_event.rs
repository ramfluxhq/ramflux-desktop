// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI events: what happened in the interface, with a bubbling scope path.
//!
//! The path runs from the root to the emitting element:
//! `[Workspace, Context, Moment, Surface, Block]` (or a `Panel` level for
//! pages, docks and overlays). A child emits an event with
//! its own scope; each parent prepends its scope with [`UiEvent::within`]
//! before passing it up. Business events bubble to the workspace root, which
//! turns them into intents (rules B1 to B5).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{
    AccountId, BlockId, CallId, ContextId, DeviceId, GrantId, MomentId, NodeId, ObjectId,
    ParticipantId, StandingApprovalId, SurfaceId,
};
use crate::intent::{ContactAction, GovernanceOp, SyncDirection};
use crate::model::{ConflictResolution, RequestAnswer};
use crate::surface_hash::SurfaceHash;

/// One level of the UI nesting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    /// The root (the workspace root receives every page's events).
    Workspace,
    /// A page, dock or overlay below the root, named by a stable key
    /// (`entry`, `settings.nodes`, `dock.members`, `palette`).
    Panel(String),
    /// A context field.
    Context(ContextId),
    /// A moment.
    Moment(MomentId),
    /// An agent surface.
    Surface(SurfaceId),
    /// A block.
    Block(BlockId),
}

/// A transfer action on an object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectAction {
    /// Download an inbound object.
    Pull,
    /// Save a downloaded object to a local path.
    Save {
        /// Local path.
        path: String,
    },
    /// Retry a failed transfer.
    Retry,
    /// Cancel a transfer on its way.
    Cancel,
}

/// A settings action on a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeAction {
    /// Probe gateway and relay.
    Probe,
    /// Remove the profile.
    Remove,
    /// Make it the default for new accounts.
    SetDefault,
    /// Enable or disable it for new accounts.
    SetEnabled(bool),
}

/// Semantic interaction kinds. Secrets never travel in UI events, and no UI
/// event can carry human confirmation: only the confirm strip adds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UiEventKind {
    /// Copy the current, still-visible message body locally; no source mutation.
    CopyMessagePressed,
    /// Retry a message whose pre-acceptance wait expired.
    RetryMessagePressed,
    /// Composer submitted text.
    SubmitMessage {
        /// Text.
        text: String,
    },
    /// Reply chosen in a message's menu: the composer quotes the message
    /// (view-local; the reply itself is [`Self::SubmitReply`]).
    ReplyPressed,
    /// Edit chosen in an own message's menu: the composer takes its text
    /// (view-local; the edit itself is [`Self::SubmitEdit`]).
    EditPressed,
    /// Delete for me (this device only) chosen in a message's menu.
    DeleteForMePressed,
    /// Delete for everyone chosen in an own message's menu.
    DeleteForEveryonePressed,
    /// The composer submitted a reply to `target`.
    SubmitReply {
        /// The message replied to.
        target: MomentId,
        /// Text.
        text: String,
    },
    /// The composer submitted the new text of `target`.
    SubmitEdit {
        /// The edited message.
        target: MomentId,
        /// Text.
        text: String,
    },
    /// Leave chosen for a group conversation.
    LeaveGroupPressed,
    /// A group invitation was accepted or declined.
    GroupInvitationAnswered {
        /// The group the invitation is for.
        group_id: String,
        /// Accept (true) or decline.
        accept: bool,
    },
    /// Add a contact by the username they claimed.
    AddContactByUsernameRequested {
        /// The name, with or without a leading `@`.
        name: String,
    },
    /// The first message of a new direct conversation was submitted.
    StartConversationRequested {
        /// The new conversation's id, chosen when the draft opened.
        context_id: ContextId,
        /// The contact.
        participant_id: ParticipantId,
        /// The first message.
        text: String,
    },
    /// A file was picked or dropped.
    AttachFile {
        /// Local path.
        path: String,
    },
    /// A surface action was invoked. The moment level fills `values` from
    /// the surface's view-local form state (rule B3).
    ActionInvoked {
        /// Action id.
        action_id: String,
        /// Collected values.
        #[serde(default)]
        values: Value,
        /// The surface version the action was rendered from.
        surface_hash: SurfaceHash,
        /// Its version number.
        #[serde(default)]
        version: u64,
    },
    /// "Request update" pressed on an out-of-sync surface.
    SurfaceUpdateRequested,
    /// A host event on one node of a card (typed input, tap, tab, modal).
    /// `seen` is the tree state the event was taken on.
    CardInteracted {
        /// Key of the node the event addresses.
        key: String,
        /// The event.
        event: crate::CardInput,
        /// The tree state the user saw.
        seen: crate::CardSeen,
    },
    /// The host's answer to a card slot (a link sheet, or an approval).
    /// `seen` is the tree state the answer was given on.
    CardSlotAnswered {
        /// Slot id, as the card named it.
        slot_id: String,
        /// The answer.
        outcome: crate::CardSlotOutcome,
        /// The tree state the user saw.
        seen: crate::CardSeen,
    },
    /// "Try again" pressed on a card whose submit did not go through: the
    /// submit the button press asked for is sent again.
    CardSubmitRetried,
    /// Approve pressed on an intervention or a task step.
    ApprovePressed,
    /// Deny pressed on an intervention or a task step.
    DenyPressed,
    /// Reveal pressed on masked content.
    RevealPressed,
    /// Conflict resolution chosen.
    ConflictResolved {
        /// Resolution.
        resolution: ConflictResolution,
    },
    /// A one-sentence request to an AI participant was submitted.
    TaskRequested {
        /// The AI participant asked.
        assignee: ParticipantId,
        /// Request text.
        text: String,
    },
    /// Start pressed on a task an AI participant proposed.
    ProposalAccepted,
    /// Cancel pressed on a task (or Decline on a proposal).
    CancelPressed,
    /// Retry pressed on a task decision that was not delivered.
    RetryDecisionPressed,
    /// Pin toggled.
    PinToggled {
        /// New pinned state.
        pinned: bool,
    },
    /// Mute toggled.
    MuteToggled {
        /// New muted state.
        muted: bool,
    },
    /// A disappearing-message timer was chosen.
    DisappearingChosen {
        /// Seconds; `None` turns it off.
        seconds: Option<u32>,
    },
    /// Contact request answered.
    ContactAnswered {
        /// Accepted or rejected.
        accept: bool,
    },
    /// A contact id was submitted from the add-contact form.
    AddContactRequested {
        /// The id the contact shared (already checked by the form).
        participant_id: ParticipantId,
    },
    /// A contact card was submitted from the add-contact form (pasted, or
    /// read from a card file).
    AddContactCardRequested {
        /// The card text (already checked by the form).
        card: String,
        /// The provisional root the user confirmed the card replaces
        /// (after the daemon asked); `None` for a plain add.
        #[serde(default)]
        replace_provisional_root: Option<String>,
    },
    /// The user asked to see this account's contact card to share it.
    ShareContactCardRequested,
    /// The new-group form was submitted.
    CreateGroupRequested {
        /// The new group conversation's id, chosen when the form opened.
        context_id: ContextId,
        /// The group's name.
        title: String,
        /// Contacts to add as members.
        members: Vec<ParticipantId>,
    },
    /// A new name was entered for the scoped group conversation.
    RenameRequested {
        /// New name.
        title: String,
    },
    /// An action on a contact link was chosen.
    ContactActionPressed {
        /// Participant.
        participant_id: ParticipantId,
        /// Action.
        action: ContactAction,
    },
    /// Verify pressed for a participant's safety number.
    VerifySafetyPressed {
        /// Participant.
        participant_id: ParticipantId,
    },
    /// Handoff offer answered.
    HandoffAnswered {
        /// Accepted or declined.
        accept: bool,
    },
    /// The call button of the scoped direct conversation was pressed.
    CallStartRequested,
    /// A ringing call was answered or declined.
    CallAnswered {
        /// The call.
        call_id: CallId,
        /// Answered (or declined).
        accept: bool,
    },
    /// Hang up pressed on a call (or cancel on one still ringing).
    CallHangUpPressed {
        /// The call.
        call_id: CallId,
    },
    /// An incoming contact request was answered.
    ContactRequestAnswered {
        /// The daemon's request id.
        request_id: String,
        /// The answer.
        answer: RequestAnswer,
    },
    /// The "accept contact requests automatically" setting was switched.
    ContactRequestPolicySwitched {
        /// On: accept every request automatically.
        auto_accept: bool,
    },
    /// "Forget old entries" was pressed on the notice that the daemon holds
    /// new contact requests back because its list of remembered requesters
    /// is full.
    ClearRequestFloorsPressed,
    /// A transfer action on an object.
    ObjectActionPressed {
        /// Object.
        object_id: ObjectId,
        /// Action.
        action: ObjectAction,
    },
    /// A governance operation was requested on the scoped context.
    GovernanceRequested {
        /// Operation.
        op: GovernanceOp,
    },
    /// Revoke pressed on an approved grant.
    RevokeGrantPressed {
        /// Grant.
        grant_id: GrantId,
    },
    /// Revoke pressed on a standing approval.
    RevokeStandingPressed {
        /// Standing approval.
        approval_id: StandingApprovalId,
    },
    /// Allow or deny pressed on an agent's approval (D40).
    AgentApprovalPressed {
        /// The account whose approvals the panel shows (the owner answering).
        owner_id: AccountId,
        /// The agent approval's id.
        approval_id: String,
        /// Allow (true) or deny (false).
        approved: bool,
    },
    /// A node address was submitted for discovery.
    AddNodeRequested {
        /// URL or node id.
        url: String,
    },
    /// Confirm pressed on a discovered node.
    ConfirmNodePressed {
        /// Node.
        node_id: NodeId,
    },
    /// A settings action on a node.
    NodeActionPressed {
        /// Node.
        node_id: NodeId,
        /// Action.
        action: NodeAction,
    },
    /// Lock pressed for an account.
    LockPressed {
        /// Account.
        account_id: AccountId,
    },
    /// The app lock switch or timeout was changed for an account.
    AppLockConfigureRequested {
        /// Account.
        account_id: AccountId,
        /// Whether the lock is on.
        enabled: bool,
        /// Timeout in milliseconds.
        timeout_ms: u64,
    },
    /// Revoke pressed on a device.
    RevokeDevicePressed {
        /// Device.
        device_id: DeviceId,
    },
    /// Own-device sync requested.
    DeviceSyncRequested {
        /// Direction.
        direction: SyncDirection,
        /// Local path.
        path: String,
    },
    /// Redacted diagnostics export requested.
    DiagnosticsExportRequested {
        /// Local path.
        path: String,
    },
    /// View-local: expand or collapse. Never reaches the store.
    ExpandToggled {
        /// New expanded state.
        expanded: bool,
    },
    /// View-local: a surface form field changed. Never reaches the store.
    FieldChanged {
        /// New value.
        value: String,
    },
    /// View-local: bring another moment of the shown context into view and
    /// open it (for example the task step that owns an intervention). Never
    /// reaches the store.
    FocusMomentRequested {
        /// The moment to show.
        moment_id: MomentId,
    },
}

/// A UI event with its bubbling path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiEvent {
    /// Root-first scope path.
    pub scope_path: Vec<Scope>,
    /// Interaction.
    pub kind: UiEventKind,
}

impl UiEvent {
    /// Builds an event emitted at `scope`.
    pub fn new(scope: Scope, kind: UiEventKind) -> Self {
        Self { scope_path: vec![scope], kind }
    }

    /// Prepends a parent scope while bubbling up.
    #[must_use]
    pub fn within(mut self, parent: Scope) -> Self {
        self.scope_path.insert(0, parent);
        self
    }

    /// Innermost context on the path.
    pub fn context_id(&self) -> Option<&ContextId> {
        self.scope_path.iter().rev().find_map(|s| match s {
            Scope::Context(id) => Some(id),
            _ => None,
        })
    }

    /// Innermost moment on the path.
    pub fn moment_id(&self) -> Option<&MomentId> {
        self.scope_path.iter().rev().find_map(|s| match s {
            Scope::Moment(id) => Some(id),
            _ => None,
        })
    }

    /// Innermost surface on the path.
    pub fn surface_id(&self) -> Option<&SurfaceId> {
        self.scope_path.iter().rev().find_map(|s| match s {
            Scope::Surface(id) => Some(id),
            _ => None,
        })
    }

    /// Innermost block on the path.
    pub fn block_id(&self) -> Option<&BlockId> {
        self.scope_path.iter().rev().find_map(|s| match s {
            Scope::Block(id) => Some(id),
            _ => None,
        })
    }

    /// Whether the event reached the workspace root.
    pub fn at_root(&self) -> bool {
        matches!(self.scope_path.first(), Some(Scope::Workspace))
    }

    /// Whether the event is view-local and must be consumed below the root (B1).
    pub fn is_view_local(&self) -> bool {
        matches!(
            self.kind,
            UiEventKind::ExpandToggled { .. }
                | UiEventKind::FieldChanged { .. }
                | UiEventKind::FocusMomentRequested { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bubbling_builds_root_first_path() {
        let event = UiEvent::new(
            Scope::Block(BlockId::new("b1")),
            UiEventKind::ActionInvoked {
                action_id: "run".into(),
                values: Value::Null,
                surface_hash: SurfaceHash::new("sha256:00"),
                version: 1,
            },
        )
        .within(Scope::Surface(SurfaceId::new("s1")))
        .within(Scope::Moment(MomentId::new("m1")))
        .within(Scope::Context(ContextId::new("c1")))
        .within(Scope::Workspace);

        assert!(event.at_root());
        assert_eq!(event.scope_path.len(), 5);
        assert_eq!(event.context_id().map(ContextId::as_str), Some("c1"));
        assert_eq!(event.moment_id().map(MomentId::as_str), Some("m1"));
        assert_eq!(event.surface_id().map(SurfaceId::as_str), Some("s1"));
        assert_eq!(event.block_id().map(BlockId::as_str), Some("b1"));
        assert!(!event.is_view_local());
    }
}
