// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Pure, read-only views derived from the store.

use std::collections::BTreeSet;

use crate::ids::{ContextId, MomentId, Timestamp};
use crate::model::{Grant, GrantState, Lifecycle, Moment, MomentKind, Object, Participant};
use crate::store::Store;

/// Maximum number of contexts in the context stack.
pub const CONTEXT_STACK_CAP: usize = 6;

/// Why a moment needs the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttentionReason {
    /// An approval decision.
    Approval,
    /// A contact request.
    ContactRequest,
    /// A task waits for input.
    TaskInput,
    /// A conflict must be resolved.
    Conflict,
    /// A key safety change must be acknowledged.
    Safety,
    /// An agent surface waits for input.
    SurfaceInput,
    /// A handoff offer.
    Handoff,
    /// Any other kind.
    Other,
}

impl AttentionReason {
    fn of(kind: &MomentKind) -> Self {
        match kind {
            MomentKind::Intervention(_) => Self::Approval,
            MomentKind::ContactRequest(_) => Self::ContactRequest,
            MomentKind::TaskProgress(_) => Self::TaskInput,
            MomentKind::Conflict(_) => Self::Conflict,
            MomentKind::SafetyChange(_) => Self::Safety,
            MomentKind::AgentSurface(_) => Self::SurfaceInput,
            MomentKind::Handoff(_) => Self::Handoff,
            MomentKind::Message(_)
            | MomentKind::ObjectTransfer(_)
            | MomentKind::Governance(_)
            | MomentKind::PrivacyReveal(_)
            | MomentKind::Fallback(_)
            | MomentKind::System(_) => Self::Other,
        }
    }
}

/// One "needs me" entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionItem {
    /// Context of the moment.
    pub context_id: ContextId,
    /// Moment in `NeedsMe`.
    pub moment_id: MomentId,
    /// Why.
    pub reason: AttentionReason,
    /// When it entered `NeedsMe`.
    pub since: Timestamp,
}

/// Grant filter for [`grants`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantFilter {
    /// Only pending grants.
    Pending,
    /// All grants.
    All,
}

/// All `NeedsMe` moments, oldest first (by `since`, then id).
pub fn attention(store: &Store) -> Vec<AttentionItem> {
    let mut items: Vec<AttentionItem> = store
        .moments()
        .filter(|m| m.lifecycle == Lifecycle::NeedsMe)
        .map(|m| AttentionItem {
            context_id: m.context_id.clone(),
            moment_id: m.id.clone(),
            reason: AttentionReason::of(&m.kind),
            since: store.needs_me_since(&m.id).unwrap_or(m.at),
        })
        .collect();
    items.sort_by(|a, b| a.since.cmp(&b.since).then_with(|| a.moment_id.cmp(&b.moment_id)));
    items
}

/// Pinned contexts in pin order, then contexts with a `NeedsMe` moment
/// (ordered by their oldest attention item), de-duplicated, capped at
/// [`CONTEXT_STACK_CAP`]. Never ordered by recent activity.
pub fn context_stack(store: &Store) -> Vec<ContextId> {
    let mut seen = BTreeSet::new();
    let pinned =
        store.pin_order().iter().filter(|id| store.context(id).is_some_and(|c| c.pinned)).cloned();
    let needing = attention(store).into_iter().map(|item| item.context_id);
    pinned.chain(needing).filter(|id| seen.insert(id.clone())).take(CONTEXT_STACK_CAP).collect()
}

/// `InProgress` moments plus task moments whose task is not terminal,
/// ordered by `(at, id)`.
pub fn in_flight(store: &Store) -> Vec<MomentId> {
    let mut moments: Vec<&Moment> = store
        .moments()
        .filter(|m| {
            m.lifecycle == Lifecycle::InProgress
                || matches!(&m.kind, MomentKind::TaskProgress(p)
                    if store.task(&p.task_id).is_some_and(|t| !t.lifecycle.is_terminal()))
        })
        .collect();
    moments.sort_by(|a, b| (a.at, &a.id).cmp(&(b.at, &b.id)));
    moments.into_iter().map(|m| m.id.clone()).collect()
}

/// Moments of one context ordered by `(at, id)`.
pub fn timeline<'a>(store: &'a Store, context_id: &ContextId) -> Vec<&'a Moment> {
    let mut moments: Vec<&Moment> =
        store.moments().filter(|m| &m.context_id == context_id).collect();
    moments.sort_by(|a, b| (a.at, &a.id).cmp(&(b.at, &b.id)));
    moments
}

/// Objects dock: all objects ordered by id.
pub fn objects(store: &Store) -> Vec<&Object> {
    store.objects().collect()
}

/// Contacts dock: participants marked as contacts, ordered by display name then id.
pub fn contacts(store: &Store) -> Vec<&Participant> {
    let mut list: Vec<&Participant> = store.participants().filter(|p| p.contact).collect();
    list.sort_by(|a, b| (&a.display, &a.id).cmp(&(&b.display, &b.id)));
    list
}

/// Grants dock, ordered by id.
pub fn grants(store: &Store, filter: GrantFilter) -> Vec<&Grant> {
    store
        .grants()
        .filter(|g| filter == GrantFilter::All || g.state == GrantState::Pending)
        .collect()
}

/// Members of a context in the context's order; unknown ids are skipped.
pub fn participants<'a>(store: &'a Store, context_id: &ContextId) -> Vec<&'a Participant> {
    store
        .context(context_id)
        .map(|c| c.participants.iter().filter_map(|id| store.participant(id)).collect())
        .unwrap_or_default()
}

/// The local user's participant: the human present in every context.
///
/// The contract has no explicit "self" field yet; every source lists the
/// local user in each of their contexts, which identifies them.
pub fn self_participant(store: &Store) -> Option<crate::ids::ParticipantId> {
    let mut contexts = store.contexts();
    let first = contexts.next()?;
    let rest: Vec<_> = contexts.collect();
    first
        .participants
        .iter()
        .filter(|id| {
            store.participant(id).is_some_and(|p| p.kind == crate::model::ParticipantKind::Human)
        })
        .find(|id| rest.iter().all(|c| c.participants.contains(id)))
        .cloned()
}
