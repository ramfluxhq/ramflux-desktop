// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Pure, read-only views derived from the store.
//!
//! Each selector reads the store's indexes, so its cost follows the size of
//! its result, not the number of moments in the store.

use std::collections::BTreeSet;

use crate::ids::{ContextId, GrantId, MomentId, ParticipantId, Timestamp};
use crate::model::{
    Call, CallState, Context, Grant, GrantState, Moment, MomentKind, Object, Participant, TaskState,
};
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
    /// A task step waits for a decision (inside the task).
    TaskInput,
    /// An AI participant proposed a task; confirm or decline it.
    TaskProposal,
    /// A conflict must be resolved.
    Conflict,
    /// A key safety change must be acknowledged.
    Safety,
    /// An agent surface waits for input.
    SurfaceInput,
    /// A handoff offer.
    Handoff,
    /// An incoming voice call rings.
    IncomingCall,
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
            MomentKind::Call(_) => Self::IncomingCall,
            MomentKind::Message(_)
            | MomentKind::ObjectTransfer(_)
            | MomentKind::Governance(_)
            | MomentKind::PrivacyReveal(_)
            | MomentKind::Fallback(_)
            | MomentKind::TaskEvent(_)
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
            let step = task.steps.iter().position(|s| s.grant_id() == Some(grant))?;
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

/// Where a task moment's decision is made, when that is an agent surface
/// (a step asking for a choice or an input): the surface's moment.
pub fn task_decision_surface<'a>(store: &'a Store, moment: &Moment) -> Option<&'a MomentId> {
    let MomentKind::TaskProgress(p) = &moment.kind else { return None };
    let task = store.task(&p.task_id)?;
    let (_, step) = task.needs_me_step()?;
    store.surface_moment(step.needs_me.as_ref()?.surface()?)
}

/// All `NeedsMe` moments, oldest first (by `since`, then id), each at the
/// one place its decision is made.
///
/// An intervention owned by a task step is not listed on its own: the task
/// moment carries the attention. A task whose step asks for a choice or an
/// input is listed at that surface instead (one moment, one place).
pub fn attention(store: &Store) -> Vec<AttentionItem> {
    store
        .needs_me()
        .filter(|(_, m)| !is_task_owned_intervention(store, m))
        .map(|(since, m)| {
            let proposal = matches!(&m.kind, MomentKind::TaskProgress(p)
                if store.task(&p.task_id).is_some_and(|t| t.state == TaskState::Proposed));
            let (moment_id, reason) = match task_decision_surface(store, m) {
                Some(surface) => (surface.clone(), AttentionReason::SurfaceInput),
                None if proposal => (m.id.clone(), AttentionReason::TaskProposal),
                None => (m.id.clone(), AttentionReason::of(&m.kind)),
            };
            AttentionItem { context_id: m.context_id.clone(), moment_id, reason, since }
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
    let running_tasks =
        store.tasks().filter(|t| !t.state.is_terminal()).flat_map(|t| store.task_presenters(&t.id));
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

/// Participants whose contact link is an incoming request, ordered by
/// display name then id.
pub fn incoming_contact_requests(store: &Store) -> Vec<&Participant> {
    linked(store, crate::model::ContactLink::Incoming)
}

/// Incoming contact requests waiting for an answer, ordered by the
/// requester's display name then id.
pub fn pending_contact_requests(store: &Store) -> Vec<&crate::model::ContactRequest> {
    let display = |r: &crate::model::ContactRequest| {
        store.participant(&r.peer).map_or_else(|| r.peer.short(), |p| p.display.clone())
    };
    let mut list: Vec<&crate::model::ContactRequest> =
        store.contact_requests().filter(|r| r.awaits_answer()).collect();
    list.sort_by_cached_key(|r| (display(r), r.peer.clone()));
    list
}

/// This account's own request to `peer`, while the peer has not accepted
/// it (pending, declined, blocked or expired).
pub fn outgoing_contact_request<'a>(
    store: &'a Store,
    peer: &ParticipantId,
) -> Option<&'a crate::model::ContactRequest> {
    store
        .contact_request(peer, crate::model::RequestDirection::Outgoing)
        .filter(|r| r.state != crate::model::RequestState::Accepted)
}

/// Blocked participants, ordered by display name then id.
pub fn blocked_contacts(store: &Store) -> Vec<&Participant> {
    linked(store, crate::model::ContactLink::Blocked)
}

fn linked(store: &Store, link: crate::model::ContactLink) -> Vec<&Participant> {
    let mut list: Vec<&Participant> =
        store.participants().filter(|p| p.link_state() == link).collect();
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

/// How a task without a result ended, and who ended it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskEnding {
    /// What ended it.
    pub how: TaskEndingKind,
    /// The member who did.
    pub by: ParticipantId,
}

/// What ended a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskEndingKind {
    /// What a step asked was refused (an approval denied).
    Denied,
    /// The task (or its proposal) was declined.
    Declined,
    /// The requester cancelled it.
    Cancelled,
}

/// Who ended a failed or cancelled task, from its signed events shown in
/// the timeline (the last refusal, decline or cancel), else from a denied
/// approval of one of its steps (the requester decides those). `None` for
/// a task still open, finished, or without either record.
pub fn task_ending(store: &Store, task: &crate::model::Task) -> Option<TaskEnding> {
    use crate::model::{GrantState, TaskEventKind, TaskState};
    if !matches!(task.state, TaskState::Failed | TaskState::Cancelled) {
        return None;
    }
    let from_event = store.context_moments(&task.context_id).rev().find_map(|m| match &m.kind {
        MomentKind::TaskEvent(p) if p.task_id == task.id => {
            let how = match p.event {
                TaskEventKind::Denied { .. } => TaskEndingKind::Denied,
                TaskEventKind::Declined => TaskEndingKind::Declined,
                TaskEventKind::Cancelled => TaskEndingKind::Cancelled,
                TaskEventKind::Accepted | TaskEventKind::Approved { .. } => return None,
            };
            Some(TaskEnding { how, by: m.origin.clone() })
        }
        _ => None,
    });
    if from_event.is_some() {
        return from_event;
    }
    let denied =
        task.step_grants().any(|g| store.grant(g).is_some_and(|g| g.state == GrantState::Denied));
    let by = match &task.requester {
        crate::model::Actor::Human => self_participant(store)?,
        crate::model::Actor::Agent(id) | crate::model::Actor::Member(id) => id.clone(),
    };
    denied.then_some(TaskEnding { how: TaskEndingKind::Denied, by })
}

/// The title a context is shown with: its own title, except that a direct
/// conversation without one (the source shows its id) is named after the
/// other participant.
pub fn context_title(store: &Store, context: &crate::model::Context) -> String {
    if context.kind != crate::model::ContextKind::Direct || context.title != context.id.as_str() {
        return context.title.clone();
    }
    let Some(me) = self_participant(store) else { return context.title.clone() };
    context
        .participants
        .iter()
        .filter(|id| **id != me)
        .find_map(|id| store.participant(id))
        .map_or_else(|| context.title.clone(), |p| p.display.clone())
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

/// The call this device is placing or in (outgoing ringing, connecting or
/// active), the most recently placed first if there were ever several.
/// Calls are few; this reads them all.
pub fn current_call(store: &Store) -> Option<&Call> {
    store
        .calls()
        .filter(|c| {
            matches!(
                c.state,
                CallState::OutgoingRinging | CallState::Connecting | CallState::Active
            )
        })
        .max_by(|a, b| (a.started_at, &a.id).cmp(&(b.started_at, &b.id)))
}

/// Incoming calls ringing on this device, the earliest first.
pub fn ringing_calls(store: &Store) -> Vec<&Call> {
    let mut calls: Vec<&Call> =
        store.calls().filter(|c| c.state == CallState::IncomingRinging).collect();
    calls.sort_by(|a, b| (a.started_at, &a.id).cmp(&(b.started_at, &b.id)));
    calls
}

/// Whether any call is under way (ringing either way, connecting or active).
pub fn call_under_way(store: &Store) -> bool {
    store.calls().any(|c| !c.state.is_ended())
}

/// The backup or restore job running (not ended), if any; a daemon runs
/// one at a time per account.
pub fn running_backup_job(store: &Store) -> Option<&crate::model::BackupJob> {
    store
        .backup_jobs()
        .filter(|j| !j.phase.is_terminal())
        .max_by(|a, b| (a.started_at, &a.id).cmp(&(b.started_at, &b.id)))
}

/// The most recently started backup or restore job, ended or not.
pub fn latest_backup_job(store: &Store) -> Option<&crate::model::BackupJob> {
    store.backup_jobs().max_by(|a, b| (a.started_at, &a.id).cmp(&(b.started_at, &b.id)))
}
