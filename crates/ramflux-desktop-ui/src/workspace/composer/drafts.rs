// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Bounded inactive composer text, erased when its source context disappears.
use ramflux_desktop_core::{ContextId, Store};
use std::collections::{BTreeMap, VecDeque};
const CAPACITY: usize = 128;
#[derive(Default)]
pub(super) struct Drafts {
    values: BTreeMap<ContextId, String>,
    recent: VecDeque<ContextId>,
}
impl Drafts {
    pub fn get(&self, id: &ContextId) -> Option<&String> {
        self.values.get(id)
    }
    pub fn remove(&mut self, id: &ContextId) {
        self.values.remove(id);
        self.recent.retain(|old| old != id);
    }
    pub fn clear(&mut self) {
        self.values.clear();
        self.recent.clear();
    }
    pub fn insert(&mut self, id: ContextId, text: String) {
        self.remove(&id);
        while self.values.len() >= CAPACITY {
            if let Some(old) = self.recent.pop_front() {
                self.values.remove(&old);
            } else {
                break;
            }
        }
        self.recent.push_back(id.clone());
        self.values.insert(id, text);
    }
    pub fn prune(&mut self, store: &Store, pending: Option<&ContextId>) {
        self.values.retain(|id, _| store.context(id).is_some() || Some(id) == pending);
        self.recent.retain(|id| self.values.contains_key(id));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inactive_plaintext_is_capped_and_removed_with_context() {
        let mut drafts = Drafts::default();
        for ix in 0..=CAPACITY {
            drafts.insert(ContextId::new(format!("draft-{ix}")), "private".into());
        }
        assert_eq!(drafts.values.len(), CAPACITY);
        assert!(
            drafts.get(&ContextId::new("draft-0")).is_none(),
            "oldest inactive draft is evicted"
        );
        let mut store = Store::new();
        let context = serde_json::from_value(
            serde_json::json!({"id":"retained","kind":"Direct","title":"Retained"}),
        )
        .expect("test: source context");
        store
            .apply(&ramflux_desktop_core::EventEnvelope::new(
                1,
                ramflux_desktop_core::Timestamp(0),
                ramflux_desktop_core::DomainEvent::ContextUpserted(context),
            ))
            .expect("test: context source");
        let retained = ContextId::new("retained");
        drafts.insert(retained.clone(), "context draft".into());
        drafts.prune(&store, None);
        assert!(drafts.get(&retained).is_some(), "a source context retains its draft");
        store
            .apply(&ramflux_desktop_core::EventEnvelope::new(
                2,
                ramflux_desktop_core::Timestamp(0),
                ramflux_desktop_core::DomainEvent::ContextRemoved { context_id: retained.clone() },
            ))
            .expect("test: source context removal");
        drafts.prune(&store, None);
        assert!(drafts.get(&retained).is_none(), "source removal erases saved plaintext");
        let pending = ContextId::new("new-context");
        drafts.insert(pending.clone(), "unsent first message".into());
        drafts.prune(&Store::new(), Some(&pending));
        assert_eq!(
            drafts.values.len(),
            1,
            "removed contexts discard their plaintext; pending first message survives"
        );
        drafts.prune(&Store::new(), None);
        assert!(drafts.values.is_empty() && drafts.recent.is_empty());
    }
}
