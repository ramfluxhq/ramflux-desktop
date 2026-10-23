// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Merged store changes and the per-area predicates views subscribe with.

use ramflux_desktop_core::{Changes, ContextId, Store};

/// Everything that changed in one batch of applied envelopes.
///
/// Views receive this through [`super::StoreEvent`] and notify themselves
/// only when their area is touched.
#[derive(Debug, Clone, Default)]
pub struct ChangeSet {
    changes: Changes,
    reset: bool,
    clock_moved: bool,
}

impl ChangeSet {
    /// A change set that invalidates every area (seek, scenario or account switch).
    pub fn reset() -> Self {
        Self { reset: true, ..Self::default() }
    }

    /// Merges the changes of one applied envelope.
    pub fn merge(&mut self, changes: Changes) {
        self.changes.merge(changes);
    }

    /// Marks that the session clock moved (only the developer panel cares).
    pub fn mark_clock(&mut self) {
        self.clock_moved = true;
    }

    /// Whether every area must refresh.
    pub fn is_reset(&self) -> bool {
        self.reset
    }

    /// Whether the session clock moved.
    pub fn is_clock_moved(&self) -> bool {
        self.clock_moved
    }

    /// Whether no domain data changed.
    pub fn is_domain_empty(&self) -> bool {
        !self.reset && self.changes.is_empty()
    }

    /// Whether nothing at all changed.
    pub fn is_empty(&self) -> bool {
        self.is_domain_empty() && !self.clock_moved
    }

    /// A collection was listed for the first time (loading became loaded).
    pub fn touches_listings(&self) -> bool {
        self.reset || self.changes.listings
    }

    /// The terminal status strip and page routing: accounts, nodes, source
    /// status, listings, and attention counts.
    pub fn touches_status(&self) -> bool {
        self.reset
            || self.changes.listings
            || !self.changes.accounts.is_empty()
            || !self.changes.nodes.is_empty()
            || !self.changes.moments.is_empty()
            || !self.changes.contexts.is_empty()
            || self.changes.source_status
    }

    /// The settings page: accounts, nodes, devices, integrations and the
    /// source status (diagnostics).
    pub fn touches_settings(&self) -> bool {
        self.reset
            || !self.changes.accounts.is_empty()
            || !self.changes.nodes.is_empty()
            || !self.changes.devices.is_empty()
            || self.changes.integrations
            || self.changes.source_status
    }

    /// The context stack: contexts, pin order, listings, and moments
    /// (attention).
    pub fn touches_context_stack(&self) -> bool {
        self.reset
            || self.changes.listings
            || !self.changes.contexts.is_empty()
            || !self.changes.moments.is_empty()
            || self.changes.pin_order
    }

    /// One conversation field. A changed moment that is no longer in the
    /// store was removed and is treated as touching every context.
    pub fn touches_context(&self, store: &Store, context_id: &ContextId) -> bool {
        self.reset
            || self.changes.contexts.contains(context_id)
            || !self.changes.participants.is_empty()
            || !self.changes.grants.is_empty()
            || !self.changes.objects.is_empty()
            || !self.changes.tasks.is_empty()
            || self
                .changes
                .moments
                .iter()
                .any(|id| store.moment(id).is_none_or(|moment| &moment.context_id == context_id))
    }

    /// The docks: objects, contacts, members, approvals and evidence.
    pub fn touches_docks(&self) -> bool {
        self.reset
            || self.changes.listings
            || !self.changes.objects.is_empty()
            || !self.changes.participants.is_empty()
            || !self.changes.grants.is_empty()
            || !self.changes.contexts.is_empty()
            || !self.changes.moments.is_empty()
            || !self.changes.devices.is_empty()
            || !self.changes.nodes.is_empty()
            || self.changes.standing_approvals
            || self.changes.audit
    }
}

#[cfg(test)]
mod tests {
    use ramflux_desktop_core::{
        Context, ContextKind, DomainEvent, EventEnvelope, Lifecycle, MessagePayload, Moment,
        MomentId, MomentKind, ParticipantId, Receipt, Timestamp,
    };
    use std::collections::BTreeMap;

    use super::*;

    fn context(id: &str) -> Context {
        Context {
            id: ContextId::new(id),
            kind: ContextKind::Direct,
            title: id.to_owned(),
            participants: Vec::new(),
            pinned: false,
            unread: 0,
            muted: false,
            disappearing_secs: None,
            roles: BTreeMap::new(),
            epoch: None,
        }
    }

    fn message(id: &str, context: &str) -> DomainEvent {
        DomainEvent::MomentAdded(Box::new(Moment {
            id: MomentId::new(id),
            context_id: ContextId::new(context),
            origin: ParticipantId::new("p-1"),
            at: Timestamp(1),
            kind: MomentKind::Message(MessagePayload {
                text: "hi".to_owned(),
                receipt: Receipt::Sent,
                tombstone: false,
                attachment: None,
                franking: None,
            }),
            lifecycle: Lifecycle::Done,
        }))
    }

    #[test]
    fn a_moment_touches_only_its_own_context() {
        let mut store = Store::new();
        let mut set = ChangeSet::default();
        for (seq, event) in [
            DomainEvent::ContextUpserted(context("c-a")),
            DomainEvent::ContextUpserted(context("c-b")),
        ]
        .into_iter()
        .enumerate()
        {
            let seq = u64::try_from(seq).unwrap_or_default() + 1;
            store.apply(&EventEnvelope::new(seq, Timestamp(1), event)).expect("apply");
        }
        let changes =
            store.apply(&EventEnvelope::new(3, Timestamp(1), message("m-1", "c-a"))).expect("ok");
        set.merge(changes);

        assert!(set.touches_context(&store, &ContextId::new("c-a")));
        assert!(!set.touches_context(&store, &ContextId::new("c-b")));
        assert!(set.touches_context_stack());
        assert!(!set.touches_settings());
        assert!(ChangeSet::reset().touches_context(&store, &ContextId::new("c-b")));
        assert!(ChangeSet::default().is_empty());
    }
}
