// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The bounded event inspector log behind the developer panel (rule B5).
//!
//! Entries are technical traces (ids, method names, contract variant names),
//! never secrets: `SecretInput` has no accessor that the inspector uses, and
//! unlock effects are recorded by label only.

use std::collections::VecDeque;
use std::time::Duration;

use ramflux_desktop_core::{DomainEvent, Effect, Intent, IntentKind, Scope, UiEvent, UiEventKind};

/// Maximum number of retained entries.
pub const INSPECTOR_CAP: usize = 200;

/// What happened to a UI event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiOutcome {
    /// Consumed below the root as view-local behavior (B1).
    Consumed {
        /// The scope that consumed it.
        at: String,
    },
    /// Reached the root and became an intent.
    Intent {
        /// Intent kind name.
        intent: String,
        /// What policy decided.
        result: String,
    },
    /// Reached the root but maps to no intent.
    Ignored,
}

/// One inspector entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectorEntry {
    /// An envelope was applied (or rejected) by the store.
    Envelope {
        /// Sequence number.
        seq: u64,
        /// Scenario offset when applied.
        at_ms: u64,
        /// Event variant name.
        event: &'static str,
        /// Rejection reason, if rejected.
        error: Option<String>,
    },
    /// A UI event bubbled.
    UiEvent {
        /// Root-first scope path.
        path: String,
        /// Event variant name.
        kind: &'static str,
        /// What happened.
        outcome: UiOutcome,
        /// Time spent handling it at the root.
        elapsed: Duration,
    },
    /// An effect was submitted to the source.
    Effect {
        /// Method name or effect kind.
        label: String,
    },
    /// An intent was refused or held for confirmation.
    Intent {
        /// Intent kind name.
        intent: &'static str,
        /// Actor label.
        actor: String,
        /// Result.
        result: String,
    },
}

/// A bounded, newest-last log.
#[derive(Debug, Default)]
pub struct Inspector {
    entries: VecDeque<InspectorEntry>,
}

impl Inspector {
    /// Appends an entry, dropping the oldest beyond [`INSPECTOR_CAP`].
    pub fn push(&mut self, entry: InspectorEntry) {
        if self.entries.len() == INSPECTOR_CAP {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// Entries, oldest first.
    pub fn entries(&self) -> impl DoubleEndedIterator<Item = &InspectorEntry> {
        self.entries.iter()
    }

    /// UI events that reached the root and became intents.
    pub fn root_intents(&self) -> impl Iterator<Item = &InspectorEntry> {
        self.entries.iter().filter(|entry| {
            matches!(entry, InspectorEntry::UiEvent { outcome: UiOutcome::Intent { .. }, .. })
        })
    }

    /// Removes every entry.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Formats a scope path as `Workspace > Context(c-1) > Moment(m-1)`.
pub fn format_path(path: &[Scope]) -> String {
    path.iter().map(format_scope).collect::<Vec<_>>().join(" > ")
}

/// Formats one scope.
pub fn format_scope(scope: &Scope) -> String {
    match scope {
        Scope::Workspace => "Workspace".to_owned(),
        Scope::Context(id) => format!("Context({id})"),
        Scope::Moment(id) => format!("Moment({id})"),
        Scope::Surface(id) => format!("Surface({id})"),
        Scope::Block(id) => format!("Block({id})"),
    }
}

/// Variant name of a UI event.
pub fn ui_event_name(event: &UiEvent) -> &'static str {
    match event.kind {
        UiEventKind::SubmitMessage { .. } => "SubmitMessage",
        UiEventKind::AttachFile { .. } => "AttachFile",
        UiEventKind::ActionInvoked { .. } => "ActionInvoked",
        UiEventKind::ApprovePressed { .. } => "ApprovePressed",
        UiEventKind::DenyPressed => "DenyPressed",
        UiEventKind::RevealPressed => "RevealPressed",
        UiEventKind::ConflictResolved { .. } => "ConflictResolved",
        UiEventKind::TaskRequested { .. } => "TaskRequested",
        UiEventKind::CancelPressed => "CancelPressed",
        UiEventKind::PinToggled { .. } => "PinToggled",
        UiEventKind::ContactAnswered { .. } => "ContactAnswered",
        UiEventKind::ExpandToggled { .. } => "ExpandToggled",
    }
}

/// Variant name of an intent.
pub fn intent_name(intent: &Intent) -> &'static str {
    match intent.kind {
        IntentKind::SendMessage { .. } => "SendMessage",
        IntentKind::AttachObject { .. } => "AttachObject",
        IntentKind::SurfaceAction { .. } => "SurfaceAction",
        IntentKind::Approve { .. } => "Approve",
        IntentKind::Deny { .. } => "Deny",
        IntentKind::Reveal { .. } => "Reveal",
        IntentKind::ResolveConflict { .. } => "ResolveConflict",
        IntentKind::StartTask { .. } => "StartTask",
        IntentKind::CancelTask { .. } => "CancelTask",
        IntentKind::PinContext { .. } => "PinContext",
        IntentKind::UnpinContext { .. } => "UnpinContext",
        IntentKind::AcceptContact { .. } => "AcceptContact",
        IntentKind::RejectContact { .. } => "RejectContact",
        IntentKind::VerifySafetyNumber { .. } => "VerifySafetyNumber",
        IntentKind::Governance { .. } => "Governance",
        IntentKind::AcceptHandoff { .. } => "AcceptHandoff",
        IntentKind::DeclineHandoff { .. } => "DeclineHandoff",
        IntentKind::Unlock { .. } => "Unlock",
    }
}

/// Label of an effect; never includes secret material.
pub fn effect_label(effect: &Effect) -> String {
    match effect {
        Effect::Call { method, .. } => method.clone(),
        Effect::RequireConfirmation { intent } => {
            format!("RequireConfirmation({})", intent_name(intent))
        }
        Effect::Unlock { account_id, .. } => format!("Unlock({account_id})"),
    }
}

/// Variant name of a domain event.
pub fn event_name(event: &DomainEvent) -> &'static str {
    match event {
        DomainEvent::AccountUpdated(_) => "AccountUpdated",
        DomainEvent::NodeUpserted(_) => "NodeUpserted",
        DomainEvent::NodeHealthChanged { .. } => "NodeHealthChanged",
        DomainEvent::ParticipantUpserted(_) => "ParticipantUpserted",
        DomainEvent::ContextUpserted(_) => "ContextUpserted",
        DomainEvent::ContextRemoved { .. } => "ContextRemoved",
        DomainEvent::MomentAdded(_) => "MomentAdded",
        DomainEvent::MomentUpdated { .. } => "MomentUpdated",
        DomainEvent::SurfacePatched { .. } => "SurfacePatched",
        DomainEvent::ObjectUpserted(_) => "ObjectUpserted",
        DomainEvent::GrantRequested(_) => "GrantRequested",
        DomainEvent::GrantResolved { .. } => "GrantResolved",
        DomainEvent::TaskUpserted(_) => "TaskUpserted",
        DomainEvent::TaskStepChanged { .. } => "TaskStepChanged",
        DomainEvent::SourceStatusChanged(_) => "SourceStatusChanged",
    }
}

#[cfg(test)]
mod tests {
    use ramflux_desktop_core::{AccountId, ContextId, SecretInput};

    use super::*;

    #[test]
    fn log_is_bounded_and_keeps_newest() {
        let mut inspector = Inspector::default();
        for ix in 0..(INSPECTOR_CAP + 5) {
            inspector.push(InspectorEntry::Effect { label: ix.to_string() });
        }
        assert_eq!(inspector.entries().count(), INSPECTOR_CAP);
        assert_eq!(
            inspector.entries().next_back(),
            Some(&InspectorEntry::Effect { label: (INSPECTOR_CAP + 4).to_string() })
        );
    }

    #[test]
    fn unlock_effects_never_show_the_secret() {
        let effect = Effect::Unlock {
            account_id: AccountId::new("a-1"),
            secret: SecretInput::new("river stone"),
        };
        assert_eq!(effect_label(&effect), "Unlock(a-1)");
    }

    #[test]
    fn paths_render_root_first() {
        let path = [Scope::Workspace, Scope::Context(ContextId::new("c-1"))];
        assert_eq!(format_path(&path), "Workspace > Context(c-1)");
    }
}
