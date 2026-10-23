// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The bounded message-body cache.
//!
//! Moment rows keep metadata only; a message's text lives here, in memory,
//! keyed by moment id. The cache holds at most its capacity of bodies and
//! evicts the least recently loaded one first (a body is "used" when the
//! source delivers it, on add, update or reload). An evicted body is not
//! lost for good: the view asks the source to deliver it again
//! (`Source::request_bodies`). Nothing here is ever written to disk, and the
//! whole cache is dropped when an account locks.
//!
//! Recency is advanced only by the reducer, never by reads, so applying the
//! same envelopes always yields the same cache (replay stays deterministic).

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::ids::MomentId;

/// Default number of message bodies kept per store.
pub const MAX_BODIES: usize = 4_096;

/// A bounded least-recently-loaded map from moment to message text.
#[derive(Debug, Clone)]
pub struct BodyCache {
    capacity: usize,
    entries: BTreeMap<MomentId, (Arc<str>, u64)>,
    order: BTreeMap<u64, MomentId>,
    tick: u64,
}

impl Default for BodyCache {
    fn default() -> Self {
        Self::with_capacity(MAX_BODIES)
    }
}

impl BodyCache {
    /// An empty cache holding at most `capacity` bodies (at least one).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: BTreeMap::new(),
            order: BTreeMap::new(),
            tick: 0,
        }
    }

    /// Stores (or refreshes) a body as the most recently loaded one,
    /// evicting the least recently loaded bodies beyond the capacity.
    pub fn insert(&mut self, id: &MomentId, text: String) {
        self.tick += 1;
        if let Some((_, old)) = self.entries.insert(id.clone(), (Arc::from(text), self.tick)) {
            self.order.remove(&old);
        }
        self.order.insert(self.tick, id.clone());
        while self.entries.len() > self.capacity {
            let Some((_, evicted)) = self.order.pop_first() else { break };
            self.entries.remove(&evicted);
        }
    }

    /// The body of `id`, if it is in memory.
    pub fn get(&self, id: &MomentId) -> Option<&Arc<str>> {
        self.entries.get(id).map(|(text, _)| text)
    }

    /// Drops the body of `id`.
    pub fn remove(&mut self, id: &MomentId) {
        if let Some((_, stamp)) = self.entries.remove(id) {
            self.order.remove(&stamp);
        }
    }

    /// Drops every body; returns whether there were any.
    pub fn clear(&mut self) -> bool {
        let any = !self.entries.is_empty();
        self.entries.clear();
        self.order.clear();
        any
    }

    /// Bodies in memory, most recently loaded first.
    pub fn recent(&self) -> impl Iterator<Item = (&MomentId, &str)> {
        self.order
            .values()
            .rev()
            .filter_map(|id| self.entries.get(id).map(|(text, _)| (id, &**text)))
    }

    /// Number of bodies in memory.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no body is in memory.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The most bodies kept.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: usize) -> MomentId {
        MomentId::new(format!("m-{n}"))
    }

    #[test]
    fn the_least_recently_loaded_body_is_evicted_first() {
        let mut cache = BodyCache::with_capacity(2);
        cache.insert(&id(1), "one".into());
        cache.insert(&id(2), "two".into());
        cache.insert(&id(1), "one again".into());
        cache.insert(&id(3), "three".into());
        assert_eq!(cache.len(), 2);
        assert!(cache.get(&id(2)).is_none(), "2 was loaded least recently");
        assert_eq!(cache.get(&id(1)).map(|t| &**t), Some("one again"));
        assert_eq!(cache.get(&id(3)).map(|t| &**t), Some("three"));
        cache.remove(&id(3));
        assert_eq!(cache.len(), 1);
        assert!(cache.clear());
        assert!(cache.is_empty() && !cache.clear());
    }

    #[test]
    fn a_large_feed_stays_bounded() {
        let mut cache = BodyCache::with_capacity(100);
        for n in 0..10_000 {
            cache.insert(&id(n), format!("body {n}"));
        }
        assert_eq!(cache.len(), 100);
        assert!(cache.get(&id(9_999)).is_some() && cache.get(&id(9_899)).is_none());
    }
}
