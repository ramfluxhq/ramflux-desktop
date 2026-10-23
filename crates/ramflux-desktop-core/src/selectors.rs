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
pub fn owning_task_step(store: &Store, grant: &crate::ids::GrantId) -> Option<TaskStepRef> {
    store.moments().find_map(|m| match &m.kind {
        MomentKind::TaskProgress(p) => {
            let task = store.task(&p.task_id)?;
            let step = task.steps.iter().position(|s| s.grant_id.as_ref() == Some(grant))?;
            Some(TaskStepRef { moment_id: m.id.clone(), task_id: task.id.clone(), step })
        }
        _ => None,
    })
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
    let mut items: Vec<AttentionItem> = store
        .moments()
        .filter(|m| m.lifecycle == Lifecycle::NeedsMe && !is_task_owned_intervention(store, m))
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
