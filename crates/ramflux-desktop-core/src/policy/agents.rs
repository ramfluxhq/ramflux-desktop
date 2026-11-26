// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for named agents (AGENT-IDENTITY-01): creating one from the
//! desktop (the account's phone approves it), editing its persona and
//! access, deleting it, its PRX jobs, the approvals it waits on, and the
//! PRX runtime on this computer.
//!
//! Every step is human only. The owner's account must be an unlocked,
//! connected person's account of the daemon. The daemon enforces the same
//! rules; the policy keeps the desktop from sending a request it knows is
//! refused. An agent never changes an agent.

use serde_json::{Value, json};

use super::{Decision, IntentError, Policy};
use crate::ids::AccountId;
use crate::intent::{Effect, IntentKind, methods};
use crate::model::{
    AgentCreateRequest, AgentCustodyProblem, AgentJobAction, AgentLifecycle, AgentPersona,
    AgentRecord, AgentRuntimeAction,
};
use crate::store::Store;

const WHAT: &str = "change an AI participant or its runtime";

/// The label of the runtime confirmation. It is not a bus method: a runtime
/// action never reaches the bus.
pub(crate) const RUNTIME_LABEL: &str = "agent.runtime.local";

/// Why `owner` cannot own an agent, if it cannot: an unlocked, connected
/// person's account the daemon lists.
///
/// # Errors
///
/// The [`AgentCustodyProblem`] that applies first.
pub fn check_agent_custodian(store: &Store, owner: &AccountId) -> Result<(), AgentCustodyProblem> {
    use crate::model::{AccountKind, LockState, Networking};
    let account = store.account(owner).ok_or(AgentCustodyProblem::Unknown)?;
    if account.kind != AccountKind::HumanInteractive {
        return Err(AgentCustodyProblem::NotPerson);
    }
    if account.lock_state != LockState::Unlocked {
        return Err(AgentCustodyProblem::Locked);
    }
    if account.networking != Networking::Live {
        return Err(AgentCustodyProblem::NotConnected);
    }
    Ok(())
}

/// Decides agent kinds; `None` for kinds of other areas.
pub(super) fn decide(p: &Policy<'_>, kind: &IntentKind) -> Option<Result<Decision, IntentError>> {
    Some(match kind {
        IntentKind::AgentCreate { owner_id, request } => p.human_only(WHAT).and_then(|()| {
            check_agent_custodian(p.store, owner_id).map_err(IntentError::AgentCustody)?;
            check_create(request)?;
            if !p.human_confirmed {
                return Ok(Decision::Confirm(methods::AGENT_CREATE));
            }
            Ok(Decision::Call(methods::AGENT_CREATE, create_body(request)?))
        }),
        IntentKind::AgentOperationRefresh { request_id } => p.human_only(WHAT).and_then(|()| {
            if request_id.trim().is_empty() {
                return Err(IntentError::Empty("request id"));
            }
            Ok(Decision::Call(methods::AGENT_OPERATION_GET, json!({ "operation_id": request_id })))
        }),
        IntentKind::AgentPersonaEdit { agent, persona } => p.human_only(WHAT).and_then(|()| {
            let record = live_record(p.store, agent)?;
            check_persona(persona)?;
            let body = json!({ "agent_anchor": record.agent_anchor, "persona": persona });
            Ok(confirmed(p, methods::AGENT_PERSONA_EDIT, body))
        }),
        IntentKind::AgentAccessSet { agent, access } => p.human_only(WHAT).and_then(|()| {
            let record = live_record(p.store, agent)?;
            if !access.is_valid() {
                return Err(IntentError::AgentAccessInvalid);
            }
            if record.base.is_null() {
                return Err(IntentError::AgentRecordBase(agent.clone()));
            }
            let body = json!({
                "agent_anchor": record.agent_anchor,
                "access": access,
                "base": record.base,
            });
            Ok(confirmed(p, methods::AGENT_ACCESS_SET, body))
        }),
        IntentKind::AgentDelete { agent } => p.human_only(WHAT).and_then(|()| {
            let record = live_record(p.store, agent)?;
            let body = json!({ "agent_anchor": record.agent_anchor });
            Ok(confirmed(p, methods::AGENT_DELETE, body))
        }),
        IntentKind::AgentJob { agent, kind, id, action } => p.human_only(WHAT).and_then(|()| {
            if id.trim().is_empty() {
                return Err(IntentError::Empty("job id"));
            }
            let method = match action {
                AgentJobAction::Pause => methods::AGENT_JOB_PAUSE,
                AgentJobAction::Resume => methods::AGENT_JOB_RESUME,
                AgentJobAction::Remove => methods::AGENT_JOB_REMOVE,
            };
            let body = json!({ "agent_anchor": agent, "kind": kind.code(), "id": id });
            if *action == AgentJobAction::Remove {
                return Ok(confirmed(p, method, body));
            }
            Ok(Decision::Call(method, body))
        }),
        IntentKind::AgentJobsRefresh { agent } => p.human_only(WHAT).and_then(|()| {
            if agent.trim().is_empty() {
                return Err(IntentError::Empty("agent anchor"));
            }
            Ok(Decision::Call(methods::AGENT_JOB_LIST, json!({ "agent_anchor": agent })))
        }),
        IntentKind::AgentApprovalDecide { approval_id, approved } => {
            p.human_only(WHAT).and_then(|()| {
                if approval_id.trim().is_empty() {
                    return Err(IntentError::Empty("approval id"));
                }
                let decision = if *approved { "approved" } else { "denied" };
                Ok(Decision::Call(
                    methods::AGENT_APPROVAL_DECIDE,
                    json!({ "approval_id": approval_id, "decision": decision }),
                ))
            })
        }
        IntentKind::AgentRuntime { action, provider, model, key } => {
            runtime(p, *action, provider, model, key.is_some())
        }
        _ => return None,
    })
}

/// A runtime action: checked, then either asked of the owner or handed to
/// the source as an in-process effect (no bus call).
fn runtime(
    p: &Policy<'_>,
    action: AgentRuntimeAction,
    provider: &str,
    model: &str,
    has_key: bool,
) -> Result<Decision, IntentError> {
    p.human_only(WHAT)?;
    check_runtime(action, provider, model, has_key)?;
    if runtime_needs_confirmation(action) && !p.human_confirmed {
        return Ok(Decision::Confirm(RUNTIME_LABEL));
    }
    Ok(Decision::Local(Effect::AgentRuntime {
        action,
        provider: provider.trim().to_owned(),
        model: model.trim().to_owned(),
        key: None,
    }))
}

/// A confirmed call: asks the human first when the proposal was not
/// confirmed, otherwise sends `confirmed: true` with the body.
fn confirmed(p: &Policy<'_>, method: &'static str, mut body: Value) -> Decision {
    if !p.human_confirmed {
        return Decision::Confirm(method);
    }
    if let Value::Object(map) = &mut body {
        map.insert("confirmed".to_owned(), Value::Bool(true));
    }
    Decision::Call(method, body)
}

/// The record of an agent the owner can change: known, not retired, and
/// its owner is an unlocked, connected person of this daemon.
fn live_record<'a>(store: &'a Store, agent: &str) -> Result<&'a AgentRecord, IntentError> {
    let record =
        store.agent(agent).ok_or_else(|| IntentError::AgentRecordUnknown(agent.to_owned()))?;
    match record.lifecycle {
        AgentLifecycle::Active => {}
        AgentLifecycle::Retired => return Err(IntentError::AgentRecordRetired(agent.to_owned())),
        AgentLifecycle::Other => return Err(IntentError::AgentRecordUnknown(agent.to_owned())),
    }
    let owner = AccountId::new(record.custodian_anchor.as_str());
    check_agent_custodian(store, &owner).map_err(IntentError::AgentCustody)?;
    Ok(record)
}

/// Whether a runtime action changes the computer's runtime and so needs
/// the owner's confirmation.
fn runtime_needs_confirmation(action: AgentRuntimeAction) -> bool {
    matches!(
        action,
        AgentRuntimeAction::Install | AgentRuntimeAction::Stop | AgentRuntimeAction::Uninstall
    )
}

fn check_runtime(
    action: AgentRuntimeAction,
    provider: &str,
    model: &str,
    has_key: bool,
) -> Result<(), IntentError> {
    match action {
        AgentRuntimeAction::Install => {
            if provider.trim().is_empty() {
                return Err(IntentError::Empty("provider"));
            }
            if model.trim().is_empty() {
                return Err(IntentError::Empty("model"));
            }
            if !has_key {
                return Err(IntentError::Empty("provider key"));
            }
            Ok(())
        }
        AgentRuntimeAction::Restart
        | AgentRuntimeAction::Stop
        | AgentRuntimeAction::Uninstall
        | AgentRuntimeAction::Status => Ok(()),
    }
}

fn check_persona(persona: &AgentPersona) -> Result<(), IntentError> {
    if persona.name.trim().is_empty() {
        return Err(IntentError::AgentFieldMissing("name"));
    }
    if persona.responsibility.trim().is_empty() {
        return Err(IntentError::AgentFieldMissing("responsibility"));
    }
    Ok(())
}

fn check_create(request: &AgentCreateRequest) -> Result<(), IntentError> {
    check_persona(&request.persona)?;
    if request.config.provider.trim().is_empty() {
        return Err(IntentError::AgentFieldMissing("provider"));
    }
    if request.config.model.trim().is_empty() {
        return Err(IntentError::AgentFieldMissing("model"));
    }
    if request.config.autonomy.trim().is_empty() {
        return Err(IntentError::AgentFieldMissing("autonomy"));
    }
    if !request.access.is_valid() {
        return Err(IntentError::AgentAccessInvalid);
    }
    Ok(())
}

/// The `agent.create` body: never `host` (the daemon refuses it at this
/// core rev), and both limits, which the daemon needs to publish the agent.
fn create_body(request: &AgentCreateRequest) -> Result<Value, IntentError> {
    let config = &request.config;
    let mut body = json!({
        "persona": request.persona,
        "config": {
            "provider": config.provider.trim(),
            "model": config.model.trim(),
            "autonomy": config.autonomy.trim(),
            "tools": config.tools.bus_list(),
            "limits": {
                "max_concurrent_turns": config.max_concurrent_turns,
                "max_cost_per_day_cents": config.max_cost_per_day_cents,
            },
        },
        "access": request.access,
        "role": "named",
        "confirmed": true,
    });
    if let Some(prompt) = config.prompt.as_deref().map(str::trim).filter(|p| !p.is_empty())
        && let Some(object) = body["config"].as_object_mut()
    {
        object.insert("prompt".to_owned(), Value::String(prompt.to_owned()));
    }
    Ok(body)
}
