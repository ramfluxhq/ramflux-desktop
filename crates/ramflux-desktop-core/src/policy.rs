// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Intent validation and policy (rules R2 and R6).
//!
//! [`handle_intent`] reads the store, never mutates it, and turns a valid
//! intent into effects for the source.

use serde_json::{Value, json};
use thiserror::Error;

use crate::ids::{AccountId, ContextId, GrantId, MomentId, ParticipantId, TaskId};
use crate::intent::{Effect, Intent, IntentKind, methods};
use crate::model::{Actor, GrantState, Lifecycle, MomentKind, ParticipantKind};
use crate::store::Store;

/// Why an intent was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IntentError {
    /// Acting agent is not a known agent participant.
    #[error("actor {0} is not a known agent")]
    UnknownAgent(ParticipantId),
    /// Agents may not perform this intent.
    #[error("agents may not {0}")]
    AgentForbidden(&'static str),
    /// Context not in the store.
    #[error("unknown context {0}")]
    UnknownContext(ContextId),
    /// Moment not in the store.
    #[error("unknown moment {0}")]
    UnknownMoment(MomentId),
    /// Grant not in the store.
    #[error("unknown grant {0}")]
    UnknownGrant(GrantId),
    /// Task not in the store.
    #[error("unknown task {0}")]
    UnknownTask(TaskId),
    /// Account not in the store.
    #[error("unknown account {0}")]
    UnknownAccount(AccountId),
    /// Text or path is empty.
    #[error("{0} must not be empty")]
    Empty(&'static str),
    /// Grant already decided.
    #[error("grant {0} is not pending")]
    GrantNotPending(GrantId),
    /// The moment is not of the kind the intent targets.
    #[error("moment {moment} is not a {expected} moment")]
    WrongMomentKind {
        /// Moment.
        moment: MomentId,
        /// Expected kind name.
        expected: &'static str,
    },
    /// The moment no longer accepts this intent.
    #[error("moment {0} is not awaiting input")]
    NotActionable(MomentId),
    /// Surface or action not found on the moment.
    #[error("unknown action {action} on moment {moment}")]
    UnknownAction {
        /// Moment.
        moment: MomentId,
        /// Action id.
        action: String,
    },
    /// Action is disabled.
    #[error("action {action} is disabled: {reason}")]
    ActionDisabled {
        /// Action id.
        action: String,
        /// Reason shown to the user.
        reason: String,
    },
    /// Task already finished.
    #[error("task {0} is already finished")]
    TaskFinished(TaskId),
    /// Context already in the requested pin state.
    #[error("context {0} is already in that pin state")]
    PinUnchanged(ContextId),
}

/// Validates `intent` against `store` and returns the effects to submit.
///
/// Policy: approving a `High` or `Critical` grant without `confirmed` yields
/// [`Effect::RequireConfirmation`]. Agent intents follow the same rules; an
/// agent cannot supply human confirmation, so for agents `confirmed` is
/// ignored on high-risk approvals, and agents may never unlock an account.
///
/// # Errors
///
/// Returns [`IntentError`] when the intent references unknown entities, targets
/// a moment in the wrong kind or state, or is not allowed for the actor.
pub fn handle_intent(store: &Store, intent: Intent) -> Result<Vec<Effect>, IntentError> {
    let actor = actor_json(store, &intent.actor)?;
    let is_agent = matches!(intent.actor, Actor::Agent(_));
    let call = |method: &str, mut body: Value| {
        if let Value::Object(map) = &mut body {
            map.insert("actor".to_owned(), actor.clone());
        }
        Ok(vec![Effect::Call { method: method.to_owned(), body }])
    };

    match &intent.kind {
        IntentKind::SendMessage { context_id, text } => {
            require_context(store, context_id)?;
            if text.trim().is_empty() {
                return Err(IntentError::Empty("message text"));
            }
            call(methods::SEND_MESSAGE, json!({ "context_id": context_id, "text": text }))
        }
        IntentKind::AttachObject { context_id, path } => {
            require_context(store, context_id)?;
            if path.is_empty() {
                return Err(IntentError::Empty("path"));
            }
            call(methods::ATTACH_OBJECT, json!({ "context_id": context_id, "path": path }))
        }
        IntentKind::SurfaceAction { moment_id, surface_id, action_id, values } => {
            let moment = require_moment(store, moment_id)?;
            let MomentKind::AgentSurface(surface) = &moment.kind else {
                return Err(IntentError::WrongMomentKind {
                    moment: moment_id.clone(),
                    expected: "AgentSurface",
                });
            };
            let action = surface
                .actions
                .iter()
                .find(|a| &surface.surface_id == surface_id && &a.id == action_id)
                .ok_or_else(|| IntentError::UnknownAction {
                    moment: moment_id.clone(),
                    action: action_id.clone(),
                })?;
            if let Some(reason) = &action.disabled_reason {
                return Err(IntentError::ActionDisabled {
                    action: action_id.clone(),
                    reason: reason.clone(),
                });
            }
            call(
                methods::SURFACE_ACTION,
                json!({
                    "moment_id": moment_id,
                    "surface_id": surface_id,
                    "action_id": action_id,
                    "values": values,
                }),
            )
        }
        IntentKind::Approve { grant_id, confirmed } => {
            let grant = require_pending_grant(store, grant_id)?;
            let human_confirmed = *confirmed && !is_agent;
            if grant.risk.requires_confirmation() && !human_confirmed {
                return Ok(vec![Effect::RequireConfirmation { intent: Box::new(intent) }]);
            }
            call(
                methods::GRANT_APPROVE,
                json!({ "grant_id": grant_id, "args_hash": grant.args_hash, "confirmed": human_confirmed }),
            )
        }
        IntentKind::Deny { grant_id } => {
            require_pending_grant(store, grant_id)?;
            call(methods::GRANT_DENY, json!({ "grant_id": grant_id }))
        }
        IntentKind::Reveal { moment_id } => {
            let moment = require_moment(store, moment_id)?;
            let MomentKind::PrivacyReveal(payload) = &moment.kind else {
                return Err(IntentError::WrongMomentKind {
                    moment: moment_id.clone(),
                    expected: "PrivacyReveal",
                });
            };
            if payload.revealed.is_some() || moment.lifecycle.is_terminal() {
                return Err(IntentError::NotActionable(moment_id.clone()));
            }
            call(methods::PRIVACY_REVEAL, json!({ "moment_id": moment_id }))
        }
        IntentKind::ResolveConflict { moment_id, resolution } => {
            let moment = require_moment(store, moment_id)?;
            let MomentKind::Conflict(payload) = &moment.kind else {
                return Err(IntentError::WrongMomentKind {
                    moment: moment_id.clone(),
                    expected: "Conflict",
                });
            };
            if payload.resolution.is_some() || moment.lifecycle.is_terminal() {
                return Err(IntentError::NotActionable(moment_id.clone()));
            }
            call(
                methods::CONFLICT_RESOLVE,
                json!({
                    "moment_id": moment_id,
                    "object_id": payload.object_id,
                    "resolution": resolution,
                }),
            )
        }
        IntentKind::StartTask { context_id, request } => {
            require_context(store, context_id)?;
            if request.trim().is_empty() {
                return Err(IntentError::Empty("task request"));
            }
            call(methods::TASK_START, json!({ "context_id": context_id, "request": request }))
        }
        IntentKind::CancelTask { task_id } => {
            let task =
                store.task(task_id).ok_or_else(|| IntentError::UnknownTask(task_id.clone()))?;
            if task.lifecycle.is_terminal() {
                return Err(IntentError::TaskFinished(task_id.clone()));
            }
            call(methods::TASK_CANCEL, json!({ "task_id": task_id }))
        }
        IntentKind::PinContext { context_id } | IntentKind::UnpinContext { context_id } => {
            let pin = matches!(intent.kind, IntentKind::PinContext { .. });
            if require_context(store, context_id)?.pinned == pin {
                return Err(IntentError::PinUnchanged(context_id.clone()));
            }
            let method = if pin { methods::CONTEXT_PIN } else { methods::CONTEXT_UNPIN };
            call(method, json!({ "context_id": context_id }))
        }
        IntentKind::AcceptContact { moment_id } | IntentKind::RejectContact { moment_id } => {
            let accept = matches!(intent.kind, IntentKind::AcceptContact { .. });
            let moment = require_moment(store, moment_id)?;
            let MomentKind::ContactRequest(payload) = &moment.kind else {
                return Err(IntentError::WrongMomentKind {
                    moment: moment_id.clone(),
                    expected: "ContactRequest",
                });
            };
            if moment.lifecycle != Lifecycle::NeedsMe {
                return Err(IntentError::NotActionable(moment_id.clone()));
            }
            let method = if accept { methods::CONTACT_ACCEPT } else { methods::CONTACT_REJECT };
            call(method, json!({ "moment_id": moment_id, "from": payload.from }))
        }
        IntentKind::Unlock { account_id, .. } => {
            if is_agent {
                return Err(IntentError::AgentForbidden("unlock an account"));
            }
            if store.account(account_id).is_none() {
                return Err(IntentError::UnknownAccount(account_id.clone()));
            }
            let IntentKind::Unlock { account_id, secret } = intent.kind else {
                unreachable!("matched Unlock above");
            };
            Ok(vec![Effect::Unlock { account_id, secret }])
        }
    }
}

fn actor_json(store: &Store, actor: &Actor) -> Result<Value, IntentError> {
    match actor {
        Actor::Human => Ok(json!("Human")),
        Actor::Agent(id) => {
            let known = store.participant(id).is_some_and(|p| p.kind == ParticipantKind::Agent);
            if known {
                Ok(json!({ "Agent": id }))
            } else {
                Err(IntentError::UnknownAgent(id.clone()))
            }
        }
    }
}

fn require_context<'a>(
    store: &'a Store,
    id: &ContextId,
) -> Result<&'a crate::model::Context, IntentError> {
    store.context(id).ok_or_else(|| IntentError::UnknownContext(id.clone()))
}

fn require_moment<'a>(
    store: &'a Store,
    id: &MomentId,
) -> Result<&'a crate::model::Moment, IntentError> {
    store.moment(id).ok_or_else(|| IntentError::UnknownMoment(id.clone()))
}

fn require_pending_grant<'a>(
    store: &'a Store,
    id: &GrantId,
) -> Result<&'a crate::model::Grant, IntentError> {
    let grant = store.grant(id).ok_or_else(|| IntentError::UnknownGrant(id.clone()))?;
    if grant.state == GrantState::Pending {
        Ok(grant)
    } else {
        Err(IntentError::GrantNotPending(id.clone()))
    }
}
