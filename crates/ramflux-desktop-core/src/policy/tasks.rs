// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for tasks: asking an AI participant, confirming its proposal,
//! cancelling.
//!
//! A task is asked of an agent member of the conversation. Confirming a
//! proposal starts the task, so only the attended human does it (agents
//! never confirm). Only the requester cancels a task or declines a
//! proposal.
//!
//! Every body names the conversation and the version the user saw, so the
//! source can tell a decision made on an older view (the task is scoped by
//! its conversation; a task id alone may repeat across conversations).
//! Declining a proposal is a cancel with `decline: true`.

use serde_json::json;

use super::{Decision, IntentError, Policy};
use crate::ids::{ContextId, ParticipantId, TaskId};
use crate::intent::{IntentKind, methods};
use crate::model::{ParticipantKind, Task, TaskAnswer, TaskState};

/// Decides task kinds; `None` for other kinds.
pub(super) fn decide(p: &Policy<'_>, kind: &IntentKind) -> Option<Result<Decision, IntentError>> {
    Some(match kind {
        IntentKind::StartTask { context_id, assignee, text } => {
            start(p, context_id, assignee, text).map(|()| {
                Decision::Call(
                    methods::TASK_START,
                    json!({
                        "conversation_id": context_id,
                        "assignee": assignee,
                        "text": text,
                        "constraints": null,
                        "proposal_id": null,
                    }),
                )
            })
        }
        IntentKind::AcceptTaskProposal { task_id } => {
            p.human_only("confirm a task proposal").and_then(|()| {
                let task = task(p, task_id)?;
                if task.state != TaskState::Proposed {
                    return Err(IntentError::NotAProposal(task_id.clone()));
                }
                requester(p, task)?;
                Ok(Decision::Call(
                    methods::TASK_START,
                    json!({
                        "conversation_id": task.context_id,
                        "assignee": task.assignee,
                        "text": task.title,
                        "constraints": null,
                        "proposal_id": task_id,
                        "version": task.version,
                    }),
                ))
            })
        }
        IntentKind::CancelTask { task_id } => task(p, task_id).and_then(|task| {
            if task.state.is_terminal() {
                return Err(IntentError::TaskFinished(task_id.clone()));
            }
            requester(p, task)?;
            Ok(Decision::Call(
                methods::TASK_CANCEL,
                json!({
                    "task_id": task_id,
                    "version": task.version,
                    "conversation_id": task.context_id,
                    "decline": task.state == TaskState::Proposed,
                }),
            ))
        }),
        IntentKind::RetryTaskDecision { task_id } => task(p, task_id).and_then(|task| {
            requester(p, task)?;
            let Some(TaskAnswer::DecisionUndelivered { grant_id, approved, .. }) = &task.answer
            else {
                return Err(IntentError::NothingToRetry(task_id.clone()));
            };
            // The decision was already made (and held to confirm when it
            // needed it): this sends the same signed decision again.
            let method = if *approved { methods::GRANT_APPROVE } else { methods::GRANT_DENY };
            Ok(Decision::Call(method, json!({ "grant_id": grant_id, "retry": true })))
        }),
        _ => return None,
    })
}

fn task<'a>(p: &Policy<'a>, id: &TaskId) -> Result<&'a Task, IntentError> {
    p.store.task(id).ok_or_else(|| IntentError::UnknownTask(id.clone()))
}

/// Only the task's requester acts on it.
pub(super) fn requester(p: &Policy<'_>, task: &Task) -> Result<(), IntentError> {
    if &task.requester == p.proposer {
        Ok(())
    } else {
        Err(IntentError::NotRequester(task.id.clone()))
    }
}

fn start(
    p: &Policy<'_>,
    context_id: &ContextId,
    assignee: &ParticipantId,
    text: &str,
) -> Result<(), IntentError> {
    let context = p.context(context_id)?;
    if text.trim().is_empty() {
        return Err(IntentError::Empty("task request"));
    }
    let agent = p
        .store
        .participant(assignee)
        .ok_or_else(|| IntentError::UnknownParticipant(assignee.clone()))?;
    if agent.kind != ParticipantKind::Agent {
        return Err(IntentError::NotAnAgent(assignee.clone()));
    }
    if !context.participants.contains(assignee) {
        return Err(IntentError::NotAMember {
            context: context_id.clone(),
            participant: assignee.clone(),
        });
    }
    Ok(())
}
