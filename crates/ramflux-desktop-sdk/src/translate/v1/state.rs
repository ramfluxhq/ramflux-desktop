// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! What the v1 projection remembers between updates.
//!
//! The text-bearing state (the messages as last reported, their authors, the
//! plaintext, preview and notification caches and the reactors) is bounded
//! like the store's body cache and dropped when the account locks. What must
//! never be forgotten is light: the ids that were deleted (a tombstone is
//! sticky, even when the delete arrives before the content), the highest
//! revision and receipt applied to each message, and the local deletes still
//! waiting for their target's author.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use ramflux_desktop_core::{Moment, MomentId};

/// How many entries each bounded map keeps: the store's own body cache size,
/// so retained messages cannot defeat that bound.
pub const RETAINED_MAX: usize = 4_096;

/// How many scalars a preview and a notification text keep.
const PREVIEW_SCALARS: usize = 80;
const NOTIFICATION_SCALARS: usize = 140;

/// A map holding at most [`RETAINED_MAX`] entries; the oldest inserted is
/// dropped first.
#[derive(Debug)]
pub(crate) struct Bounded<K, V> {
    entries: BTreeMap<K, V>,
    order: VecDeque<K>,
}

impl<K, V> Default for Bounded<K, V> {
    fn default() -> Self {
        Self { entries: BTreeMap::new(), order: VecDeque::new() }
    }
}

impl<K: Ord + Clone, V> Bounded<K, V> {
    pub fn insert(&mut self, key: K, value: V) {
        if self.entries.insert(key.clone(), value).is_some() {
            self.order.retain(|k| k != &key);
        }
        self.order.push_back(key);
        while self.entries.len() > RETAINED_MAX {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.entries.get(key)
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let removed = self.entries.remove(key);
        if removed.is_some() {
            self.order.retain(|k| k != key);
        }
        removed
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.entries.iter()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    #[cfg(feature = "test-support")]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

impl<K: Ord + Clone, V: Default> Bounded<K, V> {
    /// The entry for `key`, created empty when absent.
    pub fn get_or_default_mut(&mut self, key: &K) -> &mut V {
        if !self.entries.contains_key(key) {
            self.insert(key.clone(), V::default());
        }
        self.entries.entry(key.clone()).or_default()
    }
}

/// Reactors by reaction token.
pub(crate) type Reactors = BTreeMap<String, BTreeSet<String>>;

/// A retained message and the device that authored it (unknown for a
/// message first seen as a delete).
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub moment: Moment,
    pub author: Option<String>,
}

/// The highest revision and receipt applied to a message: light enough to
/// keep for every message, so a replay of older content cannot roll them back.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Fence {
    pub revision: u64,
    pub receipt: u8,
}

#[derive(Debug, Default)]
pub(crate) struct V1State {
    /// Messages as last reported, each with its authoring device: one
    /// entry, so the two are evicted together.
    entries: Bounded<MomentId, Entry>,
    /// Who reacted with what, per message (kept even before its content).
    pub reactions: Bounded<MomentId, Reactors>,
    plaintext: Bounded<MomentId, String>,
    preview: Bounded<MomentId, String>,
    notification: Bounded<MomentId, String>,
    /// The journal sequence last accepted for each message id (a light
    /// fence): an update with a lower one is stale.
    pub journal: BTreeMap<MomentId, u64>,
    /// Ids that were deleted: later content, edits and reactions leave them
    /// alone.
    pub tombstoned: BTreeSet<MomentId>,
    /// Highest revision and receipt applied per message.
    pub fences: BTreeMap<MomentId, Fence>,
    /// Local deletes whose target has not arrived: they apply only if that
    /// message turns out to be the account's own.
    pub pending_local: BTreeSet<MomentId>,
    /// Account anchors known to be this account's own.
    pub own_anchors: BTreeSet<String>,
}

impl V1State {
    /// The message as last reported.
    pub fn message(&self, id: &MomentId) -> Option<&Moment> {
        self.entries.get(id).map(|entry| &entry.moment)
    }

    /// The authoring device of a retained message.
    pub fn author(&self, id: &MomentId) -> Option<&str> {
        self.entries.get(id).and_then(|entry| entry.author.as_deref())
    }

    /// Retains a message with its author.
    pub fn put(&mut self, moment: Moment, author: Option<String>) {
        self.entries.insert(moment.id.clone(), Entry { moment, author });
    }

    /// Replaces a retained message's latest state, keeping its author.
    pub fn set_moment(&mut self, moment: Moment) {
        let author = self.entries.get(&moment.id).and_then(|entry| entry.author.clone());
        self.put(moment, author);
    }

    /// The retained messages.
    pub fn messages(&self) -> impl Iterator<Item = (&MomentId, &Moment)> {
        self.entries.iter().map(|(id, entry)| (id, &entry.moment))
    }

    /// Remembers the text caches of a message.
    pub fn cache_text(&mut self, id: &MomentId, text: &str) {
        self.plaintext.insert(id.clone(), text.to_owned());
        self.preview.insert(id.clone(), shorten(text, PREVIEW_SCALARS));
        self.notification.insert(id.clone(), shorten(text, NOTIFICATION_SCALARS));
    }

    /// Drops every text cache of a message.
    pub fn purge_text(&mut self, id: &MomentId) {
        self.plaintext.remove(id);
        self.preview.remove(id);
        self.notification.remove(id);
    }

    /// The fence of a message, created empty when absent.
    pub fn fence(&mut self, id: &MomentId) -> &mut Fence {
        self.fences.entry(id.clone()).or_default()
    }

    /// Drops everything that carries text (messages, authors, reactors and
    /// the caches); the light fences stay, so a replay after a lock still
    /// cannot undo a delete, an edit or a receipt.
    pub fn drop_text(&mut self) {
        self.entries.clear();
        self.reactions.clear();
        self.plaintext.clear();
        self.preview.clear();
        self.notification.clear();
    }

    /// Messages retained (a test probe).
    #[cfg(feature = "test-support")]
    pub fn retained(&self) -> usize {
        self.entries.len()
    }

    #[cfg(feature = "test-support")]
    pub fn plaintext(&self, id: &MomentId) -> Option<&str> {
        self.plaintext.get(id).map(String::as_str)
    }

    #[cfg(feature = "test-support")]
    pub fn preview(&self, id: &MomentId) -> Option<&str> {
        self.preview.get(id).map(String::as_str)
    }

    #[cfg(feature = "test-support")]
    pub fn notification(&self, id: &MomentId) -> Option<&str> {
        self.notification.get(id).map(String::as_str)
    }
}

/// The first line of `text`, at most `limit` scalars.
fn shorten(text: &str, limit: usize) -> String {
    let line = text.lines().next().unwrap_or_default();
    let mut scalars = line.chars();
    let head: String = scalars.by_ref().take(limit).collect();
    if scalars.next().is_some() { format!("{head}\u{2026}") } else { head }
}
