// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Normalized local search documents, invalidated by source context IDs.
use ramflux_desktop_core::{ContextId, MomentId, MomentKind, Store, selectors};
use std::collections::BTreeMap;
#[derive(Default)]
struct Document {
    metadata: String,
    bodies: BTreeMap<MomentId, (String, String)>,
}
#[derive(Default)]
pub(super) struct Index {
    documents: BTreeMap<ContextId, Document>,
    body_owners: BTreeMap<MomentId, ContextId>,
    pub builds: usize,
    pub body_builds: usize,
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
        self.documents.insert(id.clone(), document);
        for moment in store.context_moments(id) {
            self.update_body(store, &moment.id);
        }
        self.builds += 1;
    }
    pub fn apply(&mut self, store: &Store, changes: &crate::app::ChangeSet) {
        // Metadata changes do not invalidate the bodies of that conversation.
        for id in changes.contexts() {
            let bodies = self.documents.get_mut(id).map(|d| std::mem::take(&mut d.bodies));
            if let Some(context) = store.context(id) {
                let mut metadata =
                    format!("{}\n{}", selectors::context_title(store, context), id.as_str());
                for participant in &context.participants {
                    metadata.push('\n');
                    metadata.push_str(participant.as_str());
                    if let Some(person) = store.participant(participant) {
                        metadata.push('\n');
                        metadata.push_str(&person.display);
                    }
                }
                self.documents.insert(
                    id.clone(),
                    Document {
                        metadata: metadata.to_lowercase(),
                        bodies: bodies.unwrap_or_default(),
                    },
                );
                self.builds += 1;
            } else {
                self.documents.remove(id);
                self.body_owners.retain(|_, owner| owner != id);
            }
        }
        for id in changes.moments() {
            self.update_body(store, id);
        }
    }
    fn update_body(&mut self, store: &Store, id: &MomentId) {
        let body = store.moment(id).and_then(|moment| {
            matches!(&moment.kind, MomentKind::Message(p) if !p.tombstone)
                .then(|| store.body(id).map(|body| (&moment.context_id, body)))
                .flatten()
        });
        if let Some((owner, body)) = body {
            let Some(document) = self.documents.get_mut(owner) else {
                return;
            };
            if document.bodies.get(id).is_none_or(|(source, _)| source != body) {
                document.bodies.insert(id.clone(), (body.to_owned(), body.to_lowercase()));
                self.body_builds += 1;
            }
            self.body_owners.insert(id.clone(), owner.clone());
        } else if let Some(owner) = self.body_owners.remove(id)
            && let Some(document) = self.documents.get_mut(&owner)
        {
            document.bodies.remove(id);
        }
    }
    pub fn matches(&self, id: &ContextId, normalized: &str) -> bool {
        normalized.is_empty()
            || self.documents.get(id).is_some_and(|document| {
                document.metadata.contains(normalized)
                    || document.bodies.values().any(|(_, text)| text.contains(normalized))
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
        let builds = index.body_builds;
        let mut receipt = crate::app::ChangeSet::default();
        let receipt_event = DomainEvent::MomentUpdated {
            moment_id: MomentId::new("m-a"),
            lifecycle: None,
            kind: Some(MomentKind::Message(MessagePayload {
                text: "Private words".into(),
                receipt: ramflux_desktop_core::Receipt::Read,
                ..Default::default()
            })),
        };
        receipt.merge(
            store
                .apply(&EventEnvelope::new(4, Timestamp(0), receipt_event))
                .expect("test: actual receipt source delta"),
        );
        index.apply(&store, &receipt);
        assert_eq!(
            index.body_builds, builds,
            "a receipt-only delta must reuse normalized body text"
        );
        store
            .apply(&EventEnvelope::new(5, Timestamp(0), message("m-b", "b", "Current words")))
            .expect("test: second body");
        index.prune(&store);
        let mut appended = crate::app::ChangeSet::default();
        appended.merge(ramflux_desktop_core::Changes {
            moments: [MomentId::new("m-b")].into(),
            ..Default::default()
        });
        index.apply(&store, &appended);
        assert_eq!(index.body_builds, builds + 1, "an append normalizes only its own body");
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
            .apply(&EventEnvelope::new(6, Timestamp(0), DomainEvent::AccountUpdated(locked)))
            .expect("test: lock fact");
        index.reset(&store);
        assert!(
            !index.matches(&ContextId::new("b"), "current") && index.body_owners.is_empty(),
            "lock must discard all normalized plaintext"
        );
    }
}
