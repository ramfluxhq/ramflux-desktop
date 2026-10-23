// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI events: what happened in the interface, with a bubbling scope path.
//!
//! The path runs from the root to the emitting element:
//! `[Workspace, Context, Moment, Surface, Block]`. A child emits an event with
//! its own scope; each parent prepends its scope with [`UiEvent::within`]
//! before passing it up. Business events bubble to the workspace root, which
//! turns them into intents (rules B1 to B5).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{BlockId, ContextId, MomentId, SurfaceId};
use crate::model::ConflictResolution;

/// One level of the UI nesting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    /// The workspace root.
    Workspace,
    /// A context field.
    Context(ContextId),
    /// A moment.
    Moment(MomentId),
    /// An agent surface.
    Surface(SurfaceId),
    /// A block.
    Block(BlockId),
}

/// Semantic interaction kinds. Secrets never travel in UI events.
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
    /// A surface action was invoked.
    ActionInvoked {
        /// Action id.
        action_id: String,
        /// Collected values.
        #[serde(default)]
        values: Value,
    },
    /// Approve pressed; `confirmed` after a completed hold-to-confirm.
    ApprovePressed {
        /// Hold-to-confirm completed.
        confirmed: bool,
    },
    /// Deny pressed.
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
    /// Contact request answered.
    ContactAnswered {
        /// Accepted or rejected.
        accept: bool,
    },
    /// View-local: expand or collapse. Never reaches the store.
    ExpandToggled {
        /// New expanded state.
        expanded: bool,
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
        matches!(self.kind, UiEventKind::ExpandToggled { .. })
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
