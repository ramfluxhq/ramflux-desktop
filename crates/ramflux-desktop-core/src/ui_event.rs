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
    AccountId, BlockId, ContextId, DeviceId, GrantId, MomentId, NodeId, ObjectId, ParticipantId,
    StandingApprovalId, SurfaceId,
};
use crate::intent::{GovernanceOp, SyncDirection};
use crate::model::ConflictResolution;

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
/// event can carry human confirmation: only the hold-to-confirm strip adds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UiEventKind {
    /// Composer submitted text.
    SubmitMessage {
        /// Text.
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
    },
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
    /// A one-sentence task request was submitted.
    TaskRequested {
        /// Request text.
        request: String,
    },
    /// Cancel pressed on a task.
    CancelPressed,
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
    /// Backup export requested.
    ExportBackupRequested {
        /// Account.
        account_id: AccountId,
        /// Local path.
        path: String,
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
            UiEventKind::ActionInvoked { action_id: "run".into(), values: Value::Null },
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
