// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Conversation tasks the daemon holds, as task snapshots and task moments.
//!
//! The daemon scopes a task by (conversation, task id); the desktop task id
//! names both, and the moment presenting it is `task:<that id>`. `task.list`
//! of a conversation is read with its messages; a `task.changed` event
//! (metadata only) calls for `task.get` of that task when it names another
//! version or a marker the desktop has not shown. Each read becomes a
//! `TaskSnapshot` (the whole task at the daemon's revision, which may skip
//! revisions), and the first one of a task also adds its moment, dated by
//! the daemon's `created_at`.
//!
//! | view | desktop |
//! |---|---|
//! | `state` / step `state` `proposed` | `Proposed` / step `Pending` |
//! | step `cancelled` | step `Skipped` |
//! | `requester` this account / another / none (a proposal) | `Human` / `Member` / `Human` (this account may accept) |
//! | `needs_me` `approval{grant_id}` | the desktop grant of the approval that requested that grant |
//! | `needs_me` `choice` / `input` `{surface_id}` | that surface id |
//! | `result_ref` | the result's surface |
//! | `result_summary` (signed by the assignee, terminal only) | the task's result text |
//! | `stalled_by_assignee`, `compacted` | the task's flags |
//! | `actors.assignee.actor_kind_verified` (else the view's `actor_kind_verified`) | the flag on the assignee's kind |
//! | `pending_resolution.state` `awaiting_executor` / `executor_timeout` | `DecisionPending` / `DecisionUnconfirmed` on the open step |
//!
//! `pending_resolution` survives a restart of the daemon (and of the
//! desktop): a decision sent before either is still shown as on its way
//! until the assignee commits it (the task's next version).

use ramflux_desktop_core::{
    Actor, ContextId, DomainEvent, GrantId, Lifecycle, Moment, MomentId, MomentKind, NeedsMe,
    ParticipantId, StepId, StepState, SurfaceId, Task, TaskAnswer, TaskFlags, TaskId,
    TaskProgressPayload, TaskState, TaskStep, Timestamp,
};
use serde_json::{Value, json};

use super::{Translator, list, seconds, text};
use crate::plan::fetch::Fetch;
use crate::plan::task::TaskTarget;

/// What the source reported about one task.
#[derive(Debug, Clone)]
pub(crate) struct KnownTask {
    /// Where it lives.
    pub target: TaskTarget,
    /// The task as shown.
    pub shown: Task,
    /// Each step's `needs_me` exactly as the daemon reported it (a
    /// resolution names it back).
    pub places: Vec<(StepId, Value)>,
    /// The state of this account's resolution still waiting for the
    /// assignee (`pending_resolution.state`), as last shown.
    pub pending: Option<String>,
}

/// The desktop id of the task `task_id` in `conversation`.
pub fn task_key(conversation: &str, task_id: &str) -> TaskId {
    TaskId::new(format!("{}:{conversation}:{task_id}", conversation.len()))
}

/// The moment presenting a task.
pub fn task_moment_id(task: &TaskId) -> MomentId {
    MomentId::new(format!("task:{task}"))
}

fn task_state(value: &Value) -> Option<TaskState> {
    Some(match value.as_str()? {
        "proposed" => TaskState::Proposed,
        "running" => TaskState::Running,
        "needs_me" => TaskState::NeedsMe,
        "done" => TaskState::Done,
        "failed" => TaskState::Failed,
        "cancelled" => TaskState::Cancelled,
        _ => return None,
    })
}

fn step_state(value: &Value) -> Option<StepState> {
    Some(match value.as_str()? {
        "proposed" => StepState::Pending,
        "running" => StepState::Running,
        "needs_me" => StepState::NeedsMe,
        "done" => StepState::Done,
        "failed" => StepState::Failed,
        "cancelled" => StepState::Skipped,
        _ => return None,
    })
}

/// What a pending resolution says on the step it decides (the open one).
fn pending_answer(task: &Task, pending: Option<&str>) -> Option<TaskAnswer> {
    let step_id = task.steps.iter().find(|s| s.state == StepState::NeedsMe)?.id.clone();
    match pending? {
        "awaiting_executor" => Some(TaskAnswer::DecisionPending { step_id }),
        "executor_timeout" => Some(TaskAnswer::DecisionUnconfirmed { step_id }),
        _ => None,
    }
}

impl Translator {
    /// One page of `task.list`; the next page follows.
    pub(super) fn task_list(&mut self, context: &ContextId, value: &Value) -> super::Translated {
        let mut events = Vec::new();
        for view in list(value, "tasks") {
            events.extend(self.task_view(view));
        }
        let follow = value["next_cursor"]
            .as_str()
            .filter(|c| !c.is_empty())
            .map(|cursor| Fetch::Tasks(context.clone(), Some(cursor.to_owned())))
            .into_iter()
            .collect();
        super::Translated { events, follow }
    }

    /// One task view (`task.get`, or a row of `task.list`).
    pub(super) fn task_view(&mut self, view: &Value) -> Vec<DomainEvent> {
        let (conversation, task_id) = (text(view, "conversation_id"), text(view, "task_id"));
        if conversation.is_empty() || task_id.is_empty() {
            return Vec::new();
        }
        let context = ContextId::new(conversation);
        if !self.contexts.contains_key(&context) {
            return Vec::new();
        }
        let id = task_key(conversation, task_id);
        let Some((task, places)) = self.desktop_task(&id, &context, view) else {
            return Vec::new();
        };
        let pending = view["pending_resolution"]["state"].as_str().map(str::to_owned);
        let known = self.tasks.get(&id);
        if let Some(known) = known
            && (task.version < known.shown.version
                || (task == known.shown && pending == known.pending))
        {
            return Vec::new();
        }
        let first = known.is_none();
        let rose = known.is_some_and(|k| task.version > k.shown.version);
        let answer = pending_answer(&task, pending.as_deref());
        let target = TaskTarget { conversation: context.clone(), task_id: task_id.to_owned() };
        self.tasks.insert(id.clone(), KnownTask { target, shown: task.clone(), places, pending });
        let origin = task.assignee.clone();
        let mut events = vec![DomainEvent::TaskSnapshot(task)];
        if let Some(answer) = answer {
            events.push(DomainEvent::TaskAnswered { task_id: id.clone(), answer });
        }
        if rose && self.review_again.remove(&id) {
            let answer = TaskAnswer::Refused(ramflux_desktop_core::TaskRefusal::Changed);
            events.push(DomainEvent::TaskAnswered { task_id: id.clone(), answer });
        }
        if first {
            events.push(DomainEvent::MomentAdded(Box::new(Moment {
                id: task_moment_id(&id),
                context_id: context,
                origin,
                at: seconds(&view["created_at"]).unwrap_or(Timestamp(0)),
                kind: MomentKind::TaskProgress(TaskProgressPayload { task_id: id }),
                lifecycle: Lifecycle::InProgress,
            })));
        }
        events
    }

    fn desktop_task(
        &self,
        id: &TaskId,
        context: &ContextId,
        view: &Value,
    ) -> Option<(Task, Vec<(StepId, Value)>)> {
        let state = task_state(&view["state"])?;
        let me = self.directory.self_principal.as_deref();
        let requester = match view["requester"].as_str() {
            Some(principal) if Some(principal) == me => Actor::Human,
            Some(principal) => Actor::Member(ParticipantId::new(principal)),
            None => Actor::Human,
        };
        let mut steps = Vec::new();
        let mut places = Vec::new();
        for row in list(view, "steps") {
            let step_id = StepId::new(text(row, "id"));
            let needs_me = match text(&row["needs_me"], "kind") {
                "approval" => Some(NeedsMe::Approval {
                    grant_id: self.desktop_grant(text(&row["needs_me"], "grant_id")),
                }),
                "choice" => Some(NeedsMe::Choice {
                    surface_id: SurfaceId::new(text(&row["needs_me"], "surface_id")),
                }),
                "input" => Some(NeedsMe::Input {
                    surface_id: SurfaceId::new(text(&row["needs_me"], "surface_id")),
                }),
                _ => None,
            };
            if !row["needs_me"].is_null() {
                places.push((step_id.clone(), row["needs_me"].clone()));
            }
            steps.push(TaskStep {
                id: step_id,
                label: text(row, "title").to_owned(),
                state: step_state(&row["state"])?,
                needs_me,
            });
        }
        let task = Task {
            id: id.clone(),
            intent_id: None,
            context_id: context.clone(),
            requester,
            assignee: ParticipantId::new(text(view, "assignee")),
            title: text(view, "title").to_owned(),
            steps,
            state,
            version: view["version"].as_u64()?,
            result: view["result_summary"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .map(str::to_owned),
            result_ref: view["result_ref"].as_str().map(SurfaceId::new),
            flags: TaskFlags {
                stalled: view["stalled_by_assignee"].as_bool().unwrap_or(false),
                compacted: view["compacted"].as_bool().unwrap_or(false),
                kind_verified: view["actors"]["assignee"]["actor_kind_verified"]
                    .as_bool()
                    .or_else(|| view["actor_kind_verified"].as_bool())
                    .unwrap_or(false),
            },
            answer: None,
        };
        Some((task, places))
    }

    /// The desktop grant (approval id) of the approval that requested the
    /// daemon grant `grant`; the daemon's id itself when no approval the
    /// desktop knows names it (the step then waits for it to arrive).
    fn desktop_grant(&self, grant: &str) -> GrantId {
        let approval = self.requested_grants.get(grant).cloned().or_else(|| {
            self.daemon_grants.iter().find(|(_, g)| g.as_str() == grant).map(|(a, _)| a.clone())
        });
        GrantId::new(approval.unwrap_or_else(|| grant.to_owned()))
    }

    /// The tasks naming the daemon grant `grant` again, now that the
    /// approval that requested it is known (the step decides it in place).
    pub(super) fn tasks_naming(&mut self, grant: &str) -> Vec<DomainEvent> {
        let mut events = Vec::new();
        let ids: Vec<TaskId> = self
            .tasks
            .iter()
            .filter(|(_, k)| k.places.iter().any(|(_, p)| text(p, "grant_id") == grant))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            let Some(known) = self.tasks.get(&id) else { continue };
            let mut task = known.shown.clone();
            let desktop = self.desktop_grant(grant);
            for step in &mut task.steps {
                if let Some(NeedsMe::Approval { grant_id }) = &mut step.needs_me
                    && grant_id.as_str() == grant
                {
                    *grant_id = desktop.clone();
                }
            }
            if task != known.shown {
                if let Some(known) = self.tasks.get_mut(&id) {
                    known.shown = task.clone();
                }
                events.push(DomainEvent::TaskSnapshot(task));
            }
        }
        events
    }

    /// Whether a `task.changed` body names a task state the desktop has
    /// not shown (or one it does not know): another version, a marker, a
    /// compaction, or a change of this account's pending resolution.
    pub(super) fn task_stale(&self, body: &Value) -> bool {
        let id = task_key(text(body, "conversation_id"), text(body, "task_id"));
        let Some(known) = self.tasks.get(&id) else { return true };
        let markers =
            ["stalled_by_assignee", "resync_required", "decision_rejected", "pending", "removed"];
        body["version"].as_u64() != Some(known.shown.version)
            || markers.iter().any(|m| body[*m].as_bool() == Some(true))
            || body["compacted"].as_bool().is_some_and(|c| c != known.shown.flags.compacted)
    }

    /// The read of the task a `task.changed` body names.
    pub(super) fn task_read(body: &Value) -> Option<Fetch> {
        let (conversation, task) = (text(body, "conversation_id"), text(body, "task_id"));
        (!conversation.is_empty() && !task.is_empty())
            .then(|| Fetch::Task(ContextId::new(conversation), task.to_owned()))
    }

    /// The reads of the tasks that task message rows name and the desktop
    /// does not show yet (a row carries its `task_id`).
    pub(super) fn task_rows(&self, context: &ContextId, rows: &[&Value]) -> Vec<Fetch> {
        let mut reads: Vec<Fetch> = Vec::new();
        for row in rows {
            let task_id = text(row, "task_id");
            let read = Fetch::Task(context.clone(), task_id.to_owned());
            if !task_id.is_empty()
                && !self.tasks.contains_key(&task_key(context.as_str(), task_id))
                && !reads.contains(&read)
            {
                reads.push(read);
            }
        }
        reads
    }

    /// Where a reported task lives.
    pub(crate) fn task_target(&self, id: &str) -> Option<&TaskTarget> {
        self.tasks.get(&TaskId::new(id)).map(|k| &k.target)
    }

    /// The contract body that records the requester's decision on the
    /// approval `grant` in the task step waiting on it, if this account
    /// asked for that task. The decision is the grant flow's own (the
    /// approval id names it).
    pub fn task_resolution(&self, grant: &GrantId, approved: bool) -> Option<Value> {
        self.tasks.iter().find_map(|(id, known)| {
            if known.shown.requester != Actor::Human {
                return None;
            }
            let step = known.shown.steps.iter().find(|s| {
                s.state == StepState::NeedsMe
                    && matches!(&s.needs_me, Some(NeedsMe::Approval { grant_id }) if grant_id == grant)
            })?;
            let target = known.places.iter().find(|(s, _)| *s == step.id)?.1.clone();
            Some(json!({
                "task_id": id,
                "base_revision": known.shown.version,
                "step_id": step.id,
                "target": target,
                "decision_id": grant,
                "decision": if approved { "resume" } else { "deny" },
            }))
        })
    }
}
