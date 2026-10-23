// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The bounded event inspector log behind the developer panel (rule B5).
//!
//! Entries are technical traces (ids, method names, contract variant names,
//! see [`super::names`]), never secrets: `SecretInput` has no accessor that
//! the inspector uses, and secret calls are recorded by label only.

use std::collections::VecDeque;
use std::time::Duration;

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
    /// An intent was refused, held for confirmation, or confirmed.
    Intent {
        /// Intent kind name.
        intent: &'static str,
        /// Audit label: who proposed it and whether a human confirmed it.
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

#[cfg(test)]
mod tests {
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
}
