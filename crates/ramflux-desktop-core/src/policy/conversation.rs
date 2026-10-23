// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for messages, context settings, tasks and object transfers.

use serde_json::json;

use super::{Decision, IntentError, Policy};
use crate::ids::ObjectId;
use crate::intent::{IntentKind, methods};
use crate::model::{Direction, Object, TransferState};

/// Decides conversation-area kinds; `None` for kinds of other areas.
pub(super) fn decide(p: &Policy<'_>, kind: &IntentKind) -> Option<Result<Decision, IntentError>> {
    Some(match kind {
        IntentKind::SendMessage { context_id, text } => p.context(context_id).and_then(|_| {
            if text.trim().is_empty() {
                return Err(IntentError::Empty("message text"));
            }
            Ok(Decision::Call(
                methods::SEND_MESSAGE,
                json!({ "context_id": context_id, "text": text }),
            ))
        }),
        IntentKind::AttachObject { context_id, path } => p.context(context_id).and_then(|_| {
            non_empty(path, "path")?;
            Ok(Decision::Call(
                methods::ATTACH_OBJECT,
                json!({ "context_id": context_id, "path": path }),
            ))
        }),
        IntentKind::PinContext { context_id } | IntentKind::UnpinContext { context_id } => {
            let pin = matches!(kind, IntentKind::PinContext { .. });
            p.context(context_id).and_then(|context| {
                if context.pinned == pin {
                    return Err(IntentError::PinUnchanged(context_id.clone()));
                }
                let method = if pin { methods::CONTEXT_PIN } else { methods::CONTEXT_UNPIN };
                Ok(Decision::Call(method, json!({ "context_id": context_id })))
            })
        }
        IntentKind::MuteContext { context_id, muted } => p.context(context_id).map(|_| {
            Decision::Call(
                methods::CONTEXT_MUTE,
                json!({ "context_id": context_id, "muted": muted }),
            )
        }),
        IntentKind::SetDisappearing { context_id, seconds } => p.context(context_id).map(|_| {
            Decision::Call(
                methods::CONTEXT_DISAPPEARING,
                json!({ "context_id": context_id, "seconds": seconds }),
            )
        }),
        IntentKind::StartTask { context_id, request } => p.context(context_id).and_then(|_| {
            if request.trim().is_empty() {
                return Err(IntentError::Empty("task request"));
            }
            Ok(Decision::Call(
                methods::TASK_START,
                json!({ "context_id": context_id, "request": request }),
            ))
        }),
        IntentKind::CancelTask { task_id } => {
            p.store.task(task_id).ok_or_else(|| IntentError::UnknownTask(task_id.clone())).and_then(
                |task| {
                    if task.lifecycle.is_terminal() {
                        return Err(IntentError::TaskFinished(task_id.clone()));
                    }
                    Ok(Decision::Call(methods::TASK_CANCEL, json!({ "task_id": task_id })))
                },
            )
        }
        IntentKind::PullObject { object_id } => object(p, object_id).and_then(|object| {
            let pullable = object.direction == Direction::Inbound
                && matches!(object.transfer, TransferState::Queued);
            transfer(pullable, object_id, "be pulled", methods::OBJECT_PULL)
        }),
        IntentKind::SaveObject { object_id, path } => object(p, object_id).and_then(|object| {
            non_empty(path, "path")?;
            if !matches!(object.transfer, TransferState::Complete) {
                return Err(IntentError::TransferState {
                    object: object_id.clone(),
                    action: "be saved",
                });
            }
            Ok(Decision::Call(
                methods::OBJECT_SAVE,
                json!({ "object_id": object_id, "path": path }),
            ))
        }),
        IntentKind::RetryTransfer { object_id } => object(p, object_id).and_then(|object| {
            let failed = matches!(object.transfer, TransferState::Failed { .. });
            transfer(failed, object_id, "be retried", methods::OBJECT_RETRY)
        }),
        _ => return None,
    })
}

fn non_empty(value: &str, what: &'static str) -> Result<(), IntentError> {
    if value.trim().is_empty() { Err(IntentError::Empty(what)) } else { Ok(()) }
}

fn object<'a>(p: &Policy<'a>, id: &ObjectId) -> Result<&'a Object, IntentError> {
    p.store.object(id).ok_or_else(|| IntentError::UnknownObject(id.clone()))
}

fn transfer(
    allowed: bool,
    object_id: &ObjectId,
    action: &'static str,
    method: &'static str,
) -> Result<Decision, IntentError> {
    if allowed {
        Ok(Decision::Call(method, json!({ "object_id": object_id })))
    } else {
        Err(IntentError::TransferState { object: object_id.clone(), action })
    }
}
