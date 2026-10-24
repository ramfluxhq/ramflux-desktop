// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The daemon's answers to task effects, shown on the task they concern.
//!
//! A resolution is answered `pending: true`: the assignee confirms it once
//! it has checked the decision, so the task says the decision is on its
//! way until its next revision arrives. A refusal is shown on its task
//! (next to the controls that sent it), never as a status-strip error.
//! The refusal is the daemon's typed `error.code`; its text is never
//! parsed:
//!
//! | `error.code` | refusal |
//! |---|---|
//! | `stale_revision`, `decision_mismatch` | `Changed` (review again) |
//! | `terminal` | `Finished` |
//! | `decision_consumed` | `AlreadyDecided` |
//! | `not_requester`, `not_assignee`, `not_member`, `not_eligible_acceptor`, `human_required`, `no_local_role` | `NotAllowed` |
//! | `task_stalled` | `Stalled` |
//! | `quota_exhausted`, `rate_limited` | `Busy` |
//! | `TaskDeliveryTooLarge` | `TooLarge` |
//! | anything else | `Other` with the daemon's text |
//!
//! A task that changed since the user looked is read back, and the
//! refusal is said again on the newer task, the one to review.

use ramflux_desktop_core::{
    DomainEvent, FailureCode, StepId, TaskAnswer, TaskId, TaskRefusal, methods,
};
use serde_json::Value;

use super::{Translator, text};

/// Whether `method` is a task effect answered on its task.
pub fn answered_on_task(method: &str, body: &Value) -> bool {
    match method {
        methods::TASK_CANCEL | methods::TASK_RESOLVE => true,
        methods::TASK_START => body["proposal_id"].is_string(),
        _ => false,
    }
}

/// The refusal a daemon error stands for, by its typed code.
pub fn refusal(code: &FailureCode, message: &str) -> TaskRefusal {
    let FailureCode::Daemon(code) = code else {
        return TaskRefusal::Other(message.to_owned());
    };
    match code.as_str() {
        "stale_revision" | "decision_mismatch" => TaskRefusal::Changed,
        "terminal" => TaskRefusal::Finished,
        "decision_consumed" => TaskRefusal::AlreadyDecided,
        "not_requester"
        | "not_assignee"
        | "not_member"
        | "not_eligible_acceptor"
        | "human_required"
        | "no_local_role" => TaskRefusal::NotAllowed,
        "task_stalled" => TaskRefusal::Stalled,
        "quota_exhausted" | "rate_limited" => TaskRefusal::Busy,
        "TaskDeliveryTooLarge" => TaskRefusal::TooLarge,
        _ => TaskRefusal::Other(
            message.strip_prefix("local bus error: ").unwrap_or(message).to_owned(),
        ),
    }
}

impl Translator {
    /// The task an effect body concerns, if the desktop shows it.
    fn answered_task(&self, body: &Value) -> Option<TaskId> {
        let id = body["proposal_id"].as_str().unwrap_or_else(|| text(body, "task_id"));
        let id = TaskId::new(id);
        self.tasks.contains_key(&id).then_some(id)
    }

    /// A task effect's answer.
    pub(super) fn task_answer(
        &self,
        method: &str,
        body: &Value,
        answer: &Value,
    ) -> Vec<DomainEvent> {
        if method != methods::TASK_RESOLVE || answer["pending"].as_bool() != Some(true) {
            return Vec::new();
        }
        let Some(task_id) = self.answered_task(body) else { return Vec::new() };
        let step_id = StepId::new(text(body, "step_id"));
        vec![DomainEvent::TaskAnswered { task_id, answer: TaskAnswer::DecisionPending { step_id } }]
    }

    /// A refused task effect, on its task; empty when the task is not shown
    /// (the failure is then reported as usual).
    pub fn task_refused(
        &mut self,
        method: &str,
        body: &Value,
        code: &FailureCode,
        message: &str,
    ) -> Vec<DomainEvent> {
        if !answered_on_task(method, body) {
            return Vec::new();
        }
        let Some(task_id) = self.answered_task(body) else { return Vec::new() };
        let refusal = refusal(code, message);
        if refusal == TaskRefusal::Changed {
            // Said again on the newer task the refusal reads back, which is
            // the one to review.
            self.review_again.insert(task_id.clone());
        }
        vec![DomainEvent::TaskAnswered { task_id, answer: TaskAnswer::Refused(refusal) }]
    }

    /// A `task.changed` that says the assignee could not confirm this
    /// account's decision.
    pub(super) fn task_decision_rejected(&self, body: &Value) -> Vec<DomainEvent> {
        if body["decision_rejected"].as_bool() != Some(true) {
            return Vec::new();
        }
        let id = super::tasks::task_key(text(body, "conversation_id"), text(body, "task_id"));
        if !self.tasks.contains_key(&id) {
            return Vec::new();
        }
        let answer = TaskAnswer::Refused(TaskRefusal::DecisionRejected);
        vec![DomainEvent::TaskAnswered { task_id: id, answer }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_daemon_codes_map_to_refusals_whatever_the_text() {
        let cases = [
            ("stale_revision", TaskRefusal::Changed),
            ("decision_mismatch", TaskRefusal::Changed),
            ("terminal", TaskRefusal::Finished),
            ("decision_consumed", TaskRefusal::AlreadyDecided),
            ("not_requester", TaskRefusal::NotAllowed),
            ("not_assignee", TaskRefusal::NotAllowed),
            ("not_member", TaskRefusal::NotAllowed),
            ("not_eligible_acceptor", TaskRefusal::NotAllowed),
            ("human_required", TaskRefusal::NotAllowed),
            ("no_local_role", TaskRefusal::NotAllowed),
            ("task_stalled", TaskRefusal::Stalled),
            ("quota_exhausted", TaskRefusal::Busy),
            ("rate_limited", TaskRefusal::Busy),
            ("TaskDeliveryTooLarge", TaskRefusal::TooLarge),
        ];
        for (code, expected) in cases {
            // The text is arbitrary: only the code decides.
            let daemon = FailureCode::Daemon(code.into());
            assert_eq!(refusal(&daemon, "local bus error: requester terminal"), expected, "{code}");
        }
        let other = FailureCode::Daemon("ValidationFailed".into());
        assert_eq!(
            refusal(&other, "local bus error: expected task revision 4, received 3"),
            TaskRefusal::Other("expected task revision 4, received 3".into()),
            "old wording is not parsed"
        );
    }
}
