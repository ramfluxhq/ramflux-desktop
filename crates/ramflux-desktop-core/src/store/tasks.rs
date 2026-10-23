// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Tasks: the assignee's updates, validated before anything changes.
//!
//! Every update carries a rising version. The conversation, requester and
//! assignee of a task never change. State and step transitions follow
//! [`TaskState::can_transition`] and [`StepState::can_transition`]. A task
//! needs the requester exactly when one of its steps does, and such a step
//! names where (a grant or a surface); two steps never ask in the same
//! place. A proposal has not started.
//!
//! The moments that present a task follow its state (a proposal or a step
//! waiting on the requester makes them "needs me"), so attention derives
//! from the task and never from a second, separately sent fact.

use std::collections::BTreeSet;

use super::Store;
use super::changes::{ApplyError, Changes};
use crate::ids::{MomentId, StepId, TaskId, Timestamp};
use crate::model::{Lifecycle, MomentKind, NeedsMe, StepState, Task, TaskState};

impl Store {
    pub(super) fn upsert_task(
        &mut self,
        at: Timestamp,
        task: &Task,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        if !self.contexts.contains_key(&task.context_id) {
            return Err(ApplyError::UnknownContext(task.context_id.clone()));
        }
        validate(task)?;
        if let Some(current) = self.tasks.get(&task.id) {
            check_update(current, task)?;
        }
        self.commit_task(at, task.clone(), changes);
        Ok(())
    }

    pub(super) fn change_step(
        &mut self,
        at: Timestamp,
        step: StepChange<'_>,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        let current = self
            .tasks
            .get(step.task_id)
            .ok_or_else(|| ApplyError::UnknownTask(step.task_id.clone()))?;
        let mut next = current.clone();
        next.version = step.version;
        let (ix, _) = next.step(step.step_id).ok_or_else(|| ApplyError::UnknownStep {
            task: step.task_id.clone(),
            step: step.step_id.clone(),
        })?;
        let slot = &mut next.steps[ix];
        slot.state = step.state;
        if let Some(needs_me) = step.needs_me {
            slot.needs_me = Some(needs_me.clone());
        }
        let waiting = next.needs_me_step().is_some();
        next.state = match next.state {
            TaskState::Running if waiting => TaskState::NeedsMe,
            TaskState::NeedsMe if !waiting => TaskState::Running,
            state => state,
        };
        validate(&next)?;
        check_update(current, &next)?;
        self.commit_task(at, next, changes);
        Ok(())
    }

    pub(super) fn change_task_state(
        &mut self,
        at: Timestamp,
        update: StateChange<'_>,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        let StateChange { task_id, version, state, result } = update;
        let current =
            self.tasks.get(task_id).ok_or_else(|| ApplyError::UnknownTask(task_id.clone()))?;
        let mut next = current.clone();
        next.version = version;
        next.state = state;
        if let Some(result) = result {
            next.result = Some(result.clone());
        }
        if state.is_terminal() {
            for step in next.steps.iter_mut().filter(|s| !s.state.is_terminal()) {
                step.state = StepState::Skipped;
            }
        }
        validate(&next)?;
        check_update(current, &next)?;
        self.commit_task(at, next, changes);
        Ok(())
    }

    fn commit_task(&mut self, at: Timestamp, task: Task, changes: &mut Changes) {
        let lifecycle = task.state.moment_lifecycle();
        let presenters: Vec<MomentId> =
            self.index.task_presenters.get(&task.id).into_iter().flatten().cloned().collect();
        changes.tasks.insert(task.id.clone());
        self.index.change_task(self.tasks.get(&task.id), &task);
        self.tasks.insert(task.id.clone(), task);
        for moment in presenters {
            self.follow_task(at, &moment, lifecycle, changes);
        }
    }

    /// The lifecycle a task progress moment takes from its task, if the
    /// task is known.
    pub(super) fn task_moment_lifecycle(&self, kind: &MomentKind) -> Option<Lifecycle> {
        match kind {
            MomentKind::TaskProgress(p) => {
                self.tasks.get(&p.task_id).map(|t| t.state.moment_lifecycle())
            }
            _ => None,
        }
    }

    /// Moves a task moment to `lifecycle` (derived, so not a transition
    /// the source asked for and not checked against the moment table).
    fn follow_task(
        &mut self,
        at: Timestamp,
        moment_id: &MomentId,
        lifecycle: Lifecycle,
        changes: &mut Changes,
    ) {
        let Some(moment) = self.moments.get_mut(moment_id) else { return };
        if moment.lifecycle == lifecycle {
            return;
        }
        let old_since = if lifecycle == Lifecycle::NeedsMe {
            self.needs_me_since.insert(moment_id.clone(), at)
        } else {
            self.needs_me_since.remove(moment_id)
        };
        let new_since = (lifecycle == Lifecycle::NeedsMe).then_some(at);
        self.index.change_lifecycle(moment, (moment.lifecycle, old_since), (lifecycle, new_since));
        moment.lifecycle = lifecycle;
        changes.moments.insert(moment_id.clone());
    }
}

/// One task state update.
pub(super) struct StateChange<'a> {
    pub task_id: &'a TaskId,
    pub version: u64,
    pub state: TaskState,
    pub result: Option<&'a String>,
}

/// One step update.
pub(super) struct StepChange<'a> {
    pub task_id: &'a TaskId,
    pub version: u64,
    pub step_id: &'a StepId,
    pub state: StepState,
    pub needs_me: Option<&'a NeedsMe>,
}

fn invalid(task: &Task, problem: &'static str) -> ApplyError {
    ApplyError::InvalidTask { task: task.id.clone(), problem }
}

/// The task's own consistency.
fn validate(task: &Task) -> Result<(), ApplyError> {
    if task.version == 0 {
        return Err(invalid(task, "versions start at 1"));
    }
    let mut ids = BTreeSet::new();
    if task.steps.iter().any(|s| !ids.insert(&s.id)) {
        return Err(invalid(task, "two steps share an id"));
    }
    let waiting: Vec<&NeedsMe> = task
        .steps
        .iter()
        .filter(|s| s.state == StepState::NeedsMe)
        .map(|s| {
            s.needs_me
                .as_ref()
                .ok_or_else(|| invalid(task, "a step that needs the requester must say where"))
        })
        .collect::<Result<_, _>>()?;
    let mut places = BTreeSet::new();
    if waiting.iter().any(|place| !places.insert(*place)) {
        return Err(invalid(task, "two steps ask in the same place"));
    }
    match (task.state, waiting.is_empty()) {
        (TaskState::NeedsMe, true) => {
            Err(invalid(task, "a task that needs the requester must have a step that does"))
        }
        (state, false) if state != TaskState::NeedsMe => {
            Err(invalid(task, "only a task that needs the requester may have a step that does"))
        }
        (TaskState::Proposed, _) if task.steps.iter().any(|s| s.state != StepState::Pending) => {
            Err(invalid(task, "a proposal has not started"))
        }
        _ => Ok(()),
    }
}

/// An update against the stored task: rising version, same identity,
/// legal state and step transitions.
fn check_update(current: &Task, next: &Task) -> Result<(), ApplyError> {
    if next.version <= current.version {
        return Err(ApplyError::StaleTaskVersion {
            task: next.id.clone(),
            current: current.version,
            found: next.version,
        });
    }
    if next.context_id != current.context_id
        || next.requester != current.requester
        || next.assignee != current.assignee
    {
        return Err(ApplyError::TaskIdentityChanged(next.id.clone()));
    }
    if next.state != current.state && !TaskState::can_transition(current.state, next.state) {
        return Err(ApplyError::IllegalTaskTransition {
            task: next.id.clone(),
            from: current.state,
            to: next.state,
        });
    }
    for step in &next.steps {
        let Some((_, old)) = current.step(&step.id) else { continue };
        if step.state != old.state && !StepState::can_transition(old.state, step.state) {
            return Err(ApplyError::IllegalStepTransition {
                task: next.id.clone(),
                step: step.id.clone(),
                from: old.state,
                to: step.state,
            });
        }
    }
    Ok(())
}
