// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Who is typing: memory only, never persisted, exported or counted.
//!
//! A typing notice is visible until `min(expires_at, receipt + 5 s)`, and
//! hidden exactly at that instant. A duplicate never extends the stored
//! deadline. The store holds no typing, and this type deliberately has no
//! `Serialize`: it cannot reach a snapshot, an export, a notification or an
//! unread count by construction. The owner clears it on reconnect, lock,
//! account switch and schema-gate change.

use std::collections::BTreeMap;

use crate::ids::{ContextId, ParticipantId, Timestamp};

/// The longest a notice stays visible after it was received, in milliseconds.
pub const TYPING_MAX_MS: i64 = 5_000;

/// The longest a received notice is remembered to recognise a replay, in
/// milliseconds: until the notice itself expires, but bounded so a far
/// `expires_at` cannot silence a participant for long.
pub const TYPING_REPLAY_MS: i64 = 60_000;

/// Transient typing notices, keyed by conversation and participant.
#[derive(Debug, Clone, Default)]
pub struct TypingStore {
    /// Visible notices and when they stop being shown.
    deadlines: BTreeMap<(ContextId, ParticipantId), Timestamp>,
    /// Notices already received, until they expire themselves (bounded by
    /// [`TYPING_REPLAY_MS`]): a replay after the display cap must not make
    /// them visible again. Not a notice: it is never shown or counted.
    seen: BTreeMap<(ContextId, ParticipantId), Timestamp>,
}

impl TypingStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an `active = true` notice received at `now` that expires at
    /// `expires_at`. A notice already expired on receipt is not stored, and
    /// a duplicate of a received notice (also one already hidden by the
    /// display cap, until it expires itself) changes nothing.
    pub fn observe(
        &mut self,
        context: &ContextId,
        participant: &ParticipantId,
        now: Timestamp,
        expires_at: Timestamp,
    ) {
        self.expire(now);
        let key = (context.clone(), participant.clone());
        if self.seen.contains_key(&key) {
            return;
        }
        let deadline = expires_at.min(Timestamp(now.millis().saturating_add(TYPING_MAX_MS)));
        if deadline <= now {
            return;
        }
        let remembered = expires_at.min(Timestamp(now.millis().saturating_add(TYPING_REPLAY_MS)));
        self.seen.insert(key.clone(), remembered);
        self.deadlines.insert(key, deadline);
    }

    /// Clears one participant (an `active = false` notice).
    pub fn stop(&mut self, context: &ContextId, participant: &ParticipantId) {
        let key = (context.clone(), participant.clone());
        self.deadlines.remove(&key);
        self.seen.remove(&key);
    }

    /// Drops every notice whose deadline has been reached.
    pub fn expire(&mut self, now: Timestamp) {
        self.deadlines.retain(|_, deadline| *deadline > now);
        self.seen.retain(|_, expires_at| *expires_at > now);
    }

    /// Clears everything (reconnect, lock, account switch, gate change).
    pub fn clear(&mut self) {
        self.deadlines.clear();
        self.seen.clear();
    }

    /// The participants typing in `context` at `now`, in participant order.
    #[must_use]
    pub fn typing(&self, context: &ContextId, now: Timestamp) -> Vec<&ParticipantId> {
        self.deadlines
            .iter()
            .filter(|((c, _), deadline)| c == context && **deadline > now)
            .map(|((_, participant), _)| participant)
            .collect()
    }

    /// Visible notices, expired ones included until [`TypingStore::expire`].
    #[must_use]
    pub fn len(&self) -> usize {
        self.deadlines.len()
    }

    /// Whether no notice is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.deadlines.is_empty()
    }
}
