// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The plain-text timeline: what a terminal client shows of a conversation.
//!
//! Every moment becomes text without a renderer. An agent surface becomes
//! its fallback text (never its blocks: a client that cannot render them
//! shows what the author wrote for it), a task becomes its title, state,
//! steps, what it waits on and its result. The text is the degradation
//! contract: whatever the desktop renders, a terminal can follow the same
//! conversation from these lines.

use crate::ids::ContextId;
use crate::model::{
    GrantState, Moment, MomentKind, NeedsMe, StepState, Task, TaskEventKind, TaskState,
};
use crate::store::{Store, SurfaceState};

/// The conversation `context` as plain text, one entry per moment (an
/// entry may span several lines), oldest first.
pub fn timeline_text(store: &Store, context: &ContextId) -> Vec<String> {
    store.context_moments(context).map(|moment| moment_text(store, moment)).collect()
}

/// One moment as plain text, prefixed with its author.
pub fn moment_text(store: &Store, moment: &Moment) -> String {
    let author = store.participant(&moment.origin).map_or(moment.origin.as_str(), |p| &p.display);
    format!("{author}: {}", body(store, moment))
}

fn body(store: &Store, moment: &Moment) -> String {
    match &moment.kind {
        MomentKind::Message(p) if p.tombstone => "[message deleted]".to_owned(),
        MomentKind::Message(_) => {
            store.body(&moment.id).map_or_else(|| "[message not loaded]".to_owned(), str::to_owned)
        }
        MomentKind::AgentSurface(p) => {
            let note = match store.surface(&moment.id).map(|r| &r.state) {
                Some(SurfaceState::Closed) => " [closed]",
                Some(SurfaceState::Conflict { .. }) => " [out of date]",
                Some(SurfaceState::OverLimit(_)) => " [too large to show; summary]",
                Some(SurfaceState::OutOfSync) => " [out of sync]",
                Some(SurfaceState::Open) | None if p.streaming => " [writing]",
                Some(SurfaceState::Open) | None => "",
            };
            format!("{}{note}", p.fallback_text)
        }
        MomentKind::TaskProgress(p) => store
            .task(&p.task_id)
            .map_or_else(|| format!("[task {} not loaded]", p.task_id), |t| task_text(store, t)),
        MomentKind::Intervention(p) => {
            let state = store.grant(&p.grant_id).map_or("pending", |g| grant_state(g.state));
            format!("Approval requested ({:?} risk): {} [{state}]", p.risk, p.summary)
        }
        MomentKind::ObjectTransfer(p) => {
            let name = store
                .object(&p.object_id)
                .and_then(|o| o.name.as_deref())
                .unwrap_or(p.object_id.as_str());
            format!("[file] {name}")
        }
        MomentKind::SafetyChange(p) => format!("[safety] {:?} for {}", p.change, p.participant_id),
        MomentKind::Governance(p) => format!("[membership] {:?} {}", p.action, p.target),
        MomentKind::ContactRequest(p) => format!("[contact request] from {}", p.from),
        MomentKind::Conflict(p) => {
            format!("[conflict] {} vs {}", p.local.summary, p.remote.summary)
        }
        MomentKind::PrivacyReveal(p) => format!("[protected] {}", p.label),
        MomentKind::Handoff(p) => format!("[handoff] {}", p.summary),
        MomentKind::Fallback(p) => p.text.clone(),
        MomentKind::System(p) => format!("[system] {}", p.text),
        MomentKind::TaskEvent(p) => {
            let title = store.task(&p.task_id).map_or(p.task_id.as_str(), |t| t.title.as_str());
            let step = |id: &Option<crate::ids::StepId>| {
                id.as_ref()
                    .and_then(|id| {
                        store
                            .task(&p.task_id)?
                            .steps
                            .iter()
                            .find(|s| &s.id == id)
                            .map(|s| format!(" ({})", s.label))
                    })
                    .unwrap_or_default()
            };
            match &p.event {
                TaskEventKind::Accepted => format!("[task] accepted {title}"),
                TaskEventKind::Declined => format!("[task] declined {title}"),
                TaskEventKind::Cancelled => format!("[task] cancelled {title}"),
                TaskEventKind::Approved { step_id } => {
                    format!("[task] approved {title}{}", step(step_id))
                }
                TaskEventKind::Denied { step_id } => {
                    format!("[task] denied {title}{}", step(step_id))
                }
            }
        }
        MomentKind::Call(p) => match store.call(&p.call_id) {
            Some(call) => call_text(call),
            None => format!("[call] {}", p.call_id),
        },
        MomentKind::NameChange(p) => {
            format!("[name] {} is now {} ({})", p.old_name, p.new_name, p.participant_id)
        }
    }
}

/// One line for a voice call: its direction and how it went.
pub fn call_text(call: &crate::model::Call) -> String {
    use crate::model::{CallDirection, CallOutcome, CallState};
    let direction = match call.direction {
        CallDirection::Outgoing => "outgoing",
        CallDirection::Incoming => "incoming",
    };
    let how = match (call.state, call.outcome()) {
        (_, Some(CallOutcome::Answered { duration_ms })) => {
            let secs = duration_ms / 1000;
            format!("{}:{:02}", secs / 60, secs % 60)
        }
        (_, Some(outcome)) => format!("{outcome:?}").to_lowercase(),
        (CallState::OutgoingRinging | CallState::IncomingRinging, None) => "ringing".to_owned(),
        (CallState::Connecting, None) => "connecting".to_owned(),
        (_, None) => "in progress".to_owned(),
    };
    format!("[call] {direction}, {how}")
}

/// A task as plain text: header, steps, what it waits on, result.
pub fn task_text(store: &Store, task: &Task) -> String {
    let assignee = store.participant(&task.assignee).map_or(task.assignee.as_str(), |p| &p.display);
    let mut out = format!("Task for {assignee}: {} [{}]", task.title, task_state(task.state));
    for (ix, step) in task.steps.iter().enumerate() {
        out.push_str(&format!("\n  {}. {} [{}]", ix + 1, step.label, step_state(step.state)));
        if step.state == StepState::NeedsMe
            && let Some(needs_me) = &step.needs_me
        {
            out.push_str(&format!("\n     needs you: {}", needs_me_text(store, needs_me)));
        }
    }
    if task.state == TaskState::Proposed {
        out.push_str("\n  needs you: start or decline this proposal");
    }
    if task.flags.stalled && !task.state.is_terminal() {
        out.push_str(&format!("\n  stalled by {assignee}: it cannot send more updates"));
    }
    if task.flags.compacted {
        out.push_str("\n  history compacted: the outcome is kept");
    }
    if !task.assignee_kind_verified() {
        out.push_str("\n  (actor kind unverified)");
    }
    if let Some(result) = &task.result {
        out.push_str(&format!("\n  result: {result}"));
    } else if let Some(ending) = crate::selectors::task_ending(store, task) {
        let by = store.participant(&ending.by).map_or(ending.by.as_str(), |p| &p.display);
        out.push_str(&format!("\n  ended: {:?} by {by}", ending.how));
    }
    out
}

fn needs_me_text(store: &Store, needs_me: &NeedsMe) -> String {
    match needs_me {
        NeedsMe::Approval { grant_id } => match store.grant(grant_id) {
            Some(grant) => format!("approve {} ({:?} risk)", grant.scope, grant.risk),
            None => format!("approve {grant_id}"),
        },
        NeedsMe::Choice { surface_id } | NeedsMe::Input { surface_id } => {
            let verb = if matches!(needs_me, NeedsMe::Choice { .. }) { "choose" } else { "answer" };
            let text = store
                .surface_moment(surface_id)
                .and_then(|m| store.moment(m))
                .and_then(|m| match &m.kind {
                    MomentKind::AgentSurface(p) => Some(p.fallback_text.as_str()),
                    _ => None,
                })
                .unwrap_or(surface_id.as_str());
            format!("{verb} in \u{201c}{text}\u{201d}")
        }
    }
}

const fn task_state(state: TaskState) -> &'static str {
    match state {
        TaskState::Proposed => "proposed",
        TaskState::Running => "running",
        TaskState::NeedsMe => "needs you",
        TaskState::Done => "done",
        TaskState::Failed => "failed",
        TaskState::Cancelled => "cancelled",
    }
}

const fn step_state(state: StepState) -> &'static str {
    match state {
        StepState::Pending => "pending",
        StepState::Running => "running",
        StepState::NeedsMe => "needs you",
        StepState::Done => "done",
        StepState::Failed => "failed",
        StepState::Skipped => "skipped",
    }
}

const fn grant_state(state: GrantState) -> &'static str {
    match state {
        GrantState::Pending => "pending",
        GrantState::Approved => "approved",
        GrantState::Denied => "denied",
        GrantState::Expired => "expired",
        GrantState::Revoked => "revoked",
    }
}
