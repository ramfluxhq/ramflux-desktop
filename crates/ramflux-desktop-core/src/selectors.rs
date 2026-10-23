// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Pure, read-only views derived from the store.
//!
//! Each selector reads the store's indexes, so its cost follows the size of
//! its result, not the number of moments in the store.

use std::collections::BTreeSet;

use crate::ids::{ContextId, GrantId, MomentId, ParticipantId, Timestamp};
use crate::model::{Context, Grant, GrantState, Moment, MomentKind, Object, Participant};
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

/// Where an intervention's decision lives when a task owns its grant: the
/// task progress moment and the zero-based index of the step that waits on
/// the grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStepRef {
    /// The task progress moment presenting the task.
    pub moment_id: MomentId,
    /// The task.
    pub task_id: crate::ids::TaskId,
    /// Zero-based step index.
    pub step: usize,
}

/// The task step that owns `grant`, if a task progress moment presents a
/// task with a step waiting on it. One moment, one place: such a decision
/// renders only inside the task step, and the standalone intervention for
/// the same grant collapses to a reference.
///
/// When several task moments qualify, the one with the smallest moment id
/// owns the decision.
pub fn owning_task_step(store: &Store, grant: &GrantId) -> Option<TaskStepRef> {
    store
        .step_owners(grant)
        .filter_map(|task| {
            let step = task.steps.iter().position(|s| s.grant_id.as_ref() == Some(grant))?;
            let moment = store.task_presenters(&task.id).next()?;
            Some(TaskStepRef { moment_id: moment.id.clone(), task_id: task.id.clone(), step })
        })
        .min_by(|a, b| a.moment_id.cmp(&b.moment_id))
}

/// Whether `moment` is an intervention whose grant a task step owns.
pub fn is_task_owned_intervention(store: &Store, moment: &Moment) -> bool {
    matches!(&moment.kind, MomentKind::Intervention(p)
        if owning_task_step(store, &p.grant_id).is_some())
}

/// All `NeedsMe` moments, oldest first (by `since`, then id).
///
/// An intervention owned by a task step is not listed on its own: the task
/// moment carries the attention (one moment, one place).
pub fn attention(store: &Store) -> Vec<AttentionItem> {
    store
        .needs_me()
        .filter(|(_, m)| !is_task_owned_intervention(store, m))
        .map(|(since, m)| AttentionItem {
            context_id: m.context_id.clone(),
            moment_id: m.id.clone(),
            reason: AttentionReason::of(&m.kind),
            since,
        })
        .collect()
}

/// Number of `NeedsMe` moments in one context (as the context stack shows).
pub fn needs_me_count(store: &Store, context: &ContextId) -> usize {
    store.needs_me_in(context).count()
}

/// Open contact requests (`NeedsMe` contact request moments), oldest first.
pub fn contact_requests(store: &Store) -> Vec<&Moment> {
    store
        .needs_me()
        .map(|(_, m)| m)
        .filter(|m| matches!(m.kind, MomentKind::ContactRequest(_)))
        .collect()
}

/// The moment that presents `grant` for a decision: an intervention or an
/// approval block, or the task step that owns it; the smallest moment id
/// when several do.
pub fn presenting_moment<'a>(store: &'a Store, grant: &GrantId) -> Option<&'a Moment> {
    let direct = store.grant_presenters(grant).next();
    let owned = owning_task_step(store, grant).and_then(|step| store.moment(&step.moment_id));
    match (direct, owned) {
        (Some(a), Some(b)) => Some(if a.id <= b.id { a } else { b }),
        (a, b) => a.or(b),
    }
}

/// Contexts `participant` is a member of, by id.
pub fn contexts_with<'a>(store: &'a Store, participant: &ParticipantId) -> Vec<&'a Context> {
    store.contexts_of(participant).collect()
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
    let running_tasks = store
        .tasks()
        .filter(|t| !t.lifecycle.is_terminal())
        .flat_map(|t| store.task_presenters(&t.id));
    let mut moments: Vec<&Moment> = store.in_progress().chain(running_tasks).collect();
    moments.sort_by(|a, b| (a.at, &a.id).cmp(&(b.at, &b.id)));
    moments.dedup_by(|a, b| a.id == b.id);
    moments.into_iter().map(|m| m.id.clone()).collect()
}

/// Moments of one context ordered by `(at, id)`; O(k) in the context's size.
pub fn timeline<'a>(store: &'a Store, context_id: &ContextId) -> Vec<&'a Moment> {
    store.context_moments(context_id).collect()
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

/// The local user's participant: the `self_participant_id` the source
/// declared on the account (the unlocked one first). `None` while no account
/// declares it; the desktop never guesses it.
pub fn self_participant(store: &Store) -> Option<crate::ids::ParticipantId> {
    store
        .accounts()
        .find(|a| a.lock_state == crate::model::LockState::Unlocked)
        .or_else(|| store.accounts().next())
        .and_then(|a| a.self_participant_id.clone())
}
