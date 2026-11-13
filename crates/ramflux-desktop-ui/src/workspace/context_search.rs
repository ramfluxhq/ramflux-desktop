// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Normalized local search documents, invalidated by source context IDs.
use ramflux_desktop_core::{ContextId, MomentId, MomentKind, Store, selectors};
use std::collections::BTreeMap;
#[derive(Default)]
struct Document {
    metadata: String,
    bodies: BTreeMap<MomentId, String>,
}
#[derive(Default)]
pub(super) struct Index {
    documents: BTreeMap<ContextId, Document>,
    body_owners: BTreeMap<MomentId, ContextId>,
    pub builds: usize,
}
impl Index {
    pub fn reset(&mut self, store: &Store) {
        self.documents.clear();
        self.body_owners.clear();
        if ramflux_desktop_core::projection::conversations::is_visible(store) {
            for context in store.contexts() {
                self.update(store, &context.id);
            }
        }
    }
    /// Source-body eviction must also discard normalized plaintext. This visits
    /// only the bounded body cache, never every directory row or a query string.
    pub fn prune(&mut self, store: &Store) {
        self.body_owners.retain(|id, owner| {
            if store.body(id).is_some() {
                return true;
            }
            if let Some(document) = self.documents.get_mut(owner) {
                document.bodies.remove(id);
            }
            false
        });
    }
    pub fn update(&mut self, store: &Store, id: &ContextId) {
        if let Some(old) = self.documents.remove(id) {
            for id in old.bodies.keys() {
                self.body_owners.remove(id);
            }
        }
        if !ramflux_desktop_core::projection::conversations::is_visible(store) {
            return;
        }
        let Some(context) = store.context(id) else {
            return;
        };
        let mut document = Document {
            metadata: format!("{}\n{}", selectors::context_title(store, context), id.as_str()),
            ..Default::default()
        };
        for id in &context.participants {
            document.metadata.push('\n');
            document.metadata.push_str(id.as_str());
            if let Some(person) = store.participant(id) {
                document.metadata.push('\n');
                document.metadata.push_str(&person.display);
            }
        }
        document.metadata = document.metadata.to_lowercase();
        for moment in store.context_moments(id) {
            if matches!(&moment.kind, MomentKind::Message(p) if !p.tombstone)
                && let Some(body) = store.body(&moment.id)
            {
                document.bodies.insert(moment.id.clone(), body.to_lowercase());
                self.body_owners.insert(moment.id.clone(), id.clone());
            }
        }
        self.documents.insert(id.clone(), document);
        self.builds += 1;
    }
    pub fn matches(&self, id: &ContextId, normalized: &str) -> bool {
        normalized.is_empty()
            || self.documents.get(id).is_some_and(|document| {
                document.metadata.contains(normalized)
                    || document.bodies.values().any(|text| text.contains(normalized))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ramflux_desktop_core::{
        DomainEvent, EventEnvelope, Lifecycle, MessagePayload, Moment, ParticipantId, Timestamp,
    };
    #[test]
    fn source_eviction_and_lock_remove_normalized_plaintext() {
        let mut store = Store::with_body_capacity(1);
        for (seq, id) in [(1, "a"), (2, "b")] {
            let context =
                serde_json::from_value(serde_json::json!({"id":id,"title":id,"kind":"Direct"}))
                    .expect("test: context");
            store
                .apply(&EventEnvelope::new(
                    seq,
                    Timestamp(0),
                    DomainEvent::ContextUpserted(context),
                ))
                .expect("test: context fact");
        }
        let message = |id: &str, context: &str, text: &str| {
            DomainEvent::MomentAdded(Box::new(Moment {
                id: MomentId::new(id),
                context_id: ContextId::new(context),
                origin: ParticipantId::new("peer"),
                at: Timestamp(0),
                order: None,
                lifecycle: Lifecycle::Done,
                kind: MomentKind::Message(MessagePayload {
                    text: text.into(),
                    ..Default::default()
                }),
            }))
        };
        store
            .apply(&EventEnvelope::new(3, Timestamp(0), message("m-a", "a", "Private words")))
            .expect("test: first body");
        let mut index = Index::default();
        index.reset(&store);
        assert!(
            index.matches(&ContextId::new("a"), "private"),
            "normalized source body must initially match"
        );
        store
            .apply(&EventEnvelope::new(4, Timestamp(0), message("m-b", "b", "Current words")))
            .expect("test: second body");
        index.prune(&store);
        index.update(&store, &ContextId::new("b"));
        assert!(
            !index.matches(&ContextId::new("a"), "private"),
            "an evicted source body cannot survive in the search cache"
        );
        assert!(
            index.matches(&ContextId::new("b"), "current"),
            "current source content must remain searchable"
        );
        let locked = serde_json::from_value(
            serde_json::json!({"id":"owner","label":"Owner","lock_state":"Locked"}),
        )
        .expect("test: locked account");
        store
            .apply(&EventEnvelope::new(5, Timestamp(0), DomainEvent::AccountUpdated(locked)))
            .expect("test: lock fact");
        index.reset(&store);
        assert!(
            !index.matches(&ContextId::new("b"), "current") && index.body_owners.is_empty(),
            "lock must discard all normalized plaintext"
        );
    }
}
