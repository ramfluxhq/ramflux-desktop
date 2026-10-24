// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Task effects to the daemon's task methods.
//!
//! | contract | daemon |
//! |---|---|
//! | `task.start` | `task.start {conversation_id, assignee, text, task_id, message_id}` (fresh ids) |
//! | `task.start` with `proposal_id` | `task.accept {conversation_id, task_id, version + 1, message_id}` (the daemon's only acceptance) |
//! | `task.cancel` | `task.cancel {conversation_id, task_id, version, message_id}` (the version seen) |
//! | `task.cancel` with `decline` | `task.decline {conversation_id, task_id, version + 1, message_id}` |
//! | `task.resolve` | `task.resolve {conversation_id, task_id, base_revision, step_id, target, decision_id, decision, message_id}` |
//!
//! A desktop task id names a task within its conversation (the daemon
//! scopes tasks by both); the daemon's own task id comes from what the
//! translator reported. Accept and decline carry the version the user saw
//! plus one, and cancel and resolve the version they saw, so a decision
//! made on an older view is refused by the daemon ("changed, review
//! again") instead of applying to a task the user did not see. After each
//! change the task is read again.

use serde_json::{Value, json};

use super::fetch::Fetch;
use super::message::fresh_id;
use super::{Known, Plan, refuse_invalid, text};
use crate::bus::BusRequest;
use ramflux_desktop_core::{ContextId, methods};

/// Where a reported task lives: its conversation and the daemon's id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskTarget {
    /// The conversation.
    pub conversation: ContextId,
    /// The daemon's task id (unique within the conversation).
    pub task_id: String,
}

fn version(body: &Value) -> Option<u64> {
    body["version"].as_u64()
}

/// Plans one task effect.
pub fn task_call(acct: Option<&str>, method: &str, body: &Value, known: &impl Known) -> Plan {
    let Ok(message_id) = fresh_id("desk-task-msg-") else {
        return refuse_invalid("no randomness for a message id");
    };
    let request = |daemon: &str, mut request: Value, target: &TaskTarget| {
        request["conversation_id"] = json!(target.conversation);
        request["task_id"] = json!(target.task_id);
        request["message_id"] = json!(message_id);
        Plan::Call {
            request: BusRequest::new(acct, "task", daemon, request),
            refresh: vec![Fetch::Task(target.conversation.clone(), target.task_id.clone())],
        }
    };
    let known_target = |field: &str| known.task(text(body, field)).cloned();
    match method {
        methods::TASK_START if body["proposal_id"].is_string() => {
            let (Some(target), Some(shown)) = (known_target("proposal_id"), version(body)) else {
                return refuse_invalid("unknown task proposal");
            };
            request("task.accept", json!({ "version": shown + 1 }), &target)
        }
        methods::TASK_START => {
            let Ok(task_id) = fresh_id("desk-task-") else {
                return refuse_invalid("no randomness for a task id");
            };
            let target =
                TaskTarget { conversation: ContextId::new(text(body, "conversation_id")), task_id };
            let start = json!({ "assignee": body["assignee"], "text": body["text"] });
            request("task.start", start, &target)
        }
        methods::TASK_CANCEL => {
            let (Some(target), Some(shown)) = (known_target("task_id"), version(body)) else {
                return refuse_invalid("unknown task");
            };
            if body["decline"].as_bool() == Some(true) {
                request("task.decline", json!({ "version": shown + 1 }), &target)
            } else {
                request("task.cancel", json!({ "version": shown }), &target)
            }
        }
        methods::TASK_RESOLVE => {
            let Some(target) = known_target("task_id") else {
                return refuse_invalid("unknown task");
            };
            let resolve = json!({
                "base_revision": body["base_revision"],
                "step_id": body["step_id"],
                "target": body["target"],
                "decision_id": body["decision_id"],
                "decision": body["decision"],
            });
            request("task.resolve", resolve, &target)
        }
        _ => refuse_invalid("no daemon counterpart"),
    }
}
