// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Named agents (AGENT-IDENTITY-01): the daemon bodies of the agent intents.
//!
//! Every body is built field by field from an allow-list, so the owner's
//! audit stamps (`proposed_by`, `confirmed_by`) and any `host` never reach
//! the daemon (it refuses unknown fields, and a host binding is not offered
//! at this core rev). Mutations carry `confirmed: true` only when the
//! desktop has the owner's confirmation (the core asks for it first). A
//! PRX job names its agent by the runtime name derived from the anchor.

use std::time::Duration;

use ramflux_desktop_core::{AccountId, AgentRuntimeAction, Store, methods};
use ramflux_prx_client::AgentName;
use serde_json::{Map, Value, json};

use super::fetch::Fetch;
use super::{Plan, refuse_invalid, text};
use crate::bus::BusRequest;

/// The `sdk_api` label of the agent requests.
pub const API: &str = "agent";

/// How often the reads of a creation still waiting are repeated.
pub const CREATION_POLL: Duration = Duration::from_secs(2);

/// The reads that follow each creation request still waiting for the
/// phone or the publication: one `agent.operation.get` each. A caller that
/// holds the store repeats them every [`CREATION_POLL`] while any is pending.
pub fn pending_creation_reads(store: &Store) -> Vec<Fetch> {
    store
        .agent_creations()
        .filter(|creation| !creation.state.is_terminal())
        .map(|creation| Fetch::AgentOperation(creation.request_id.clone()))
        .collect()
}

/// The label of a runtime action in the plan and the effect log. It is not a
/// bus method: a runtime action never reaches the bus.
pub const RUNTIME_METHOD: &str = "agent.runtime.local";

/// The name of a runtime action, as the effect body carries it.
pub fn runtime_action_name(action: AgentRuntimeAction) -> &'static str {
    match action {
        AgentRuntimeAction::Install => "install",
        AgentRuntimeAction::Restart => "restart",
        AgentRuntimeAction::Stop => "stop",
        AgentRuntimeAction::Uninstall => "uninstall",
        AgentRuntimeAction::Status => "status",
    }
}

/// Whether `method` is an agent request this module plans.
pub fn is_agent_method(method: &str) -> bool {
    matches!(
        method,
        methods::AGENT_CREATE
            | methods::AGENT_OPERATION_GET
            | methods::AGENT_PERSONA_EDIT
            | methods::AGENT_ACCESS_SET
            | methods::AGENT_DELETE
            | methods::AGENT_JOB_PAUSE
            | methods::AGENT_JOB_RESUME
            | methods::AGENT_JOB_REMOVE
            | methods::AGENT_APPROVAL_DECIDE
    )
}

/// The PRX runtime name of the agent with `anchor` (its base64url anchor).
///
/// # Errors
///
/// A message when the anchor is not 32 bytes of base64url.
pub fn prx_name(anchor: &str) -> Result<String, String> {
    let bytes = ramflux_protocol::decode_base64url(anchor)
        .map_err(|_| "the agent anchor is not base64url".to_owned())?;
    let array: [u8; 32] =
        bytes.try_into().map_err(|_| "the agent anchor is not 32 bytes".to_owned())?;
    Ok(AgentName::from_anchor(&array).as_str().to_owned())
}

/// Plans one agent intent for the owner `account`.
pub fn agent_effect(method: &str, body: &Value, account: Option<&AccountId>) -> Plan {
    let Some(owner) = account.map(AccountId::as_str) else {
        return refuse_invalid("an agent request names the owner's account");
    };
    let built = match method {
        methods::AGENT_CREATE => create(body),
        methods::AGENT_OPERATION_GET => {
            Ok((json!({ "operation_id": text(body, "operation_id") }), vec![]))
        }
        methods::AGENT_PERSONA_EDIT => anchored(body).map(|(anchor, mut out)| {
            out.insert("agent_anchor".into(), json!(anchor));
            out.insert("persona".into(), body["persona"].clone());
            confirmed(out, vec![Fetch::AgentList])
        }),
        methods::AGENT_ACCESS_SET => anchored(body).and_then(|(anchor, mut out)| {
            if !body["base"].is_object() {
                return Err("the access change names the record it was read from".to_owned());
            }
            out.insert("agent_anchor".into(), json!(anchor));
            out.insert("access".into(), body["access"].clone());
            out.insert("base".into(), body["base"].clone());
            Ok(confirmed(out, vec![Fetch::AgentList]))
        }),
        methods::AGENT_DELETE => anchored(body).map(|(anchor, mut out)| {
            out.insert("agent_anchor".into(), json!(anchor));
            confirmed(out, vec![Fetch::AgentList])
        }),
        methods::AGENT_JOB_PAUSE | methods::AGENT_JOB_RESUME | methods::AGENT_JOB_REMOVE => {
            job(method, body)
        }
        methods::AGENT_APPROVAL_DECIDE => {
            let decision = match body["decision"].as_str() {
                Some("approved") => "approved",
                Some("denied") => "denied",
                _ => return refuse_invalid("the approval decision is approved or denied"),
            };
            let approval = text(body, "approval_id");
            if approval.is_empty() {
                return refuse_invalid("the approval names its id");
            }
            Ok((
                json!({ "approval_id": approval, "decision": decision }),
                vec![Fetch::AgentApprovals],
            ))
        }
        _ => return refuse_invalid("no daemon counterpart"),
    };
    match built {
        Ok((body, refresh)) => {
            Plan::Call { request: BusRequest::new(Some(owner), API, method, body), refresh }
        }
        Err(message) => refuse_invalid(&message),
    }
}

/// The creation body: persona, runtime configuration, access and the role.
/// Both limits are always sent; `host` is never sent.
fn create(body: &Value) -> Result<(Value, Vec<Fetch>), String> {
    if !body["confirmed"].as_bool().unwrap_or(false) {
        return Err("the creation needs the owner's confirmation".to_owned());
    }
    let config = &body["config"];
    let mut out = Map::new();
    out.insert("persona".into(), body["persona"].clone());
    out.insert(
        "config".into(),
        json!({
            "provider": config["provider"],
            "model": config["model"],
            "autonomy": config["autonomy"],
            "tools": config["tools"],
            "prompt": config.get("prompt").cloned().unwrap_or(Value::Null),
            "limits": config["limits"],
        }),
    );
    out.insert("access".into(), body["access"].clone());
    out.insert("role".into(), json!("named"));
    out.insert("confirmed".into(), json!(true));
    Ok((Value::Object(out), vec![Fetch::AgentList]))
}

/// The record anchor of a mutation, with an empty body to fill.
fn anchored(body: &Value) -> Result<(String, Map<String, Value>), String> {
    let anchor = text(body, "agent_anchor");
    if anchor.is_empty() {
        return Err("the agent request names its agent".to_owned());
    }
    Ok((anchor.to_owned(), Map::new()))
}

/// Adds `confirmed: true` to a mutation body and its refresh reads.
fn confirmed(mut out: Map<String, Value>, refresh: Vec<Fetch>) -> (Value, Vec<Fetch>) {
    out.insert("confirmed".into(), json!(true));
    (Value::Object(out), refresh)
}

/// A job request: the PRX name of its agent, the kind and the id.
fn job(method: &str, body: &Value) -> Result<(Value, Vec<Fetch>), String> {
    let (anchor, _) = anchored(body)?;
    let name = prx_name(&anchor)?;
    let kind = match body["kind"].as_str() {
        Some(kind @ ("cron" | "xin_task" | "xin_goal")) => kind,
        _ => return Err("the job kind is cron, xin_task or xin_goal".to_owned()),
    };
    let id = text(body, "id");
    if id.is_empty() {
        return Err("the job names its id".to_owned());
    }
    let mut out = json!({ "agent": name, "kind": kind, "id": id });
    if method == methods::AGENT_JOB_REMOVE {
        out["confirmed"] = json!(true);
    }
    Ok((out, vec![Fetch::AgentJobs(anchor)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANCHOR: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn owner() -> AccountId {
        AccountId::new("acct-main")
    }

    fn request(plan: Plan) -> (BusRequest, Vec<Fetch>) {
        match plan {
            Plan::Call { request, refresh } => (request, refresh),
            other => panic!("test: expected a call, got {other:?}"),
        }
    }

    #[test]
    fn creation_sends_both_limits_confirmed_and_never_a_host() {
        let body = json!({
            "persona": {"name": "N", "avatar": "preset:spark:teal", "responsibility": "R"},
            "config": {
                "provider": "anthropic", "model": "m", "autonomy": "full", "tools": [],
                "limits": {"max_concurrent_turns": 2, "max_cost_per_day_cents": 500},
                "prompt": null,
            },
            "access": {"dm": "owner", "group_inviters": "owner", "allowed_humans": []},
            "role": "named", "confirmed": true, "host": {"device": "x"},
            "proposed_by": "Human", "confirmed_by": {"gesture": "Tap"},
        });
        let (request, refresh) =
            request(agent_effect(methods::AGENT_CREATE, &body, Some(&owner())));
        assert_eq!(request.method, "agent.create");
        assert_eq!(request.account.as_deref(), Some("acct-main"));
        assert!(request.body.get("host").is_none());
        assert!(request.body.get("proposed_by").is_none());
        assert_eq!(request.body["confirmed"], true);
        assert_eq!(request.body["config"]["limits"]["max_cost_per_day_cents"], 500);
        assert_eq!(refresh, vec![Fetch::AgentList]);
    }

    #[test]
    fn creation_without_the_owners_confirmation_is_not_sent() {
        let body = json!({"persona": {}, "config": {}, "access": {}, "confirmed": false});
        assert!(matches!(
            agent_effect(methods::AGENT_CREATE, &body, Some(&owner())),
            Plan::Refuse { .. }
        ));
    }

    #[test]
    fn access_change_sends_the_base_unchanged() {
        let base = json!({"id": "b", "hlc": 7});
        let body = json!({
            "agent_anchor": ANCHOR,
            "access": {"dm": "contacts", "group_inviters": "owner", "allowed_humans": []},
            "base": base,
        });
        let (request, _) = request(agent_effect(methods::AGENT_ACCESS_SET, &body, Some(&owner())));
        assert_eq!(request.body["base"], base);
        assert_eq!(request.body["confirmed"], true);
        assert_eq!(request.body["agent_anchor"], ANCHOR);
    }

    #[test]
    fn approval_decision_is_the_daemon_word() {
        let body = json!({"approval_id": "ap-1", "approved": true, "decision": "denied"});
        let (request, refresh) =
            request(agent_effect(methods::AGENT_APPROVAL_DECIDE, &body, Some(&owner())));
        assert_eq!(request.body, json!({"approval_id": "ap-1", "decision": "denied"}));
        assert_eq!(refresh, vec![Fetch::AgentApprovals]);
    }

    #[test]
    fn job_bodies_name_the_agent_by_its_prx_name() {
        let body = json!({"agent_anchor": ANCHOR, "kind": "xin_goal", "id": "j-1"});
        let (request, refresh) =
            request(agent_effect(methods::AGENT_JOB_PAUSE, &body, Some(&owner())));
        let name = prx_name(ANCHOR).expect("test: a 32-byte anchor");
        assert!(name.starts_with("a_"), "rf-derived names carry the rf prefix");
        assert_eq!(request.body, json!({"agent": name, "kind": "xin_goal", "id": "j-1"}));
        assert_eq!(refresh, vec![Fetch::AgentJobs(ANCHOR.to_owned())]);
    }

    #[test]
    fn a_malformed_anchor_is_refused_before_any_request() {
        let body = json!({"agent_anchor": "not-an-anchor", "kind": "cron", "id": "j"});
        assert!(matches!(
            agent_effect(methods::AGENT_JOB_PAUSE, &body, Some(&owner())),
            Plan::Refuse { .. }
        ));
        assert!(prx_name("not-an-anchor").is_err());
    }

    #[test]
    fn an_agent_request_without_the_owner_is_not_sent() {
        assert!(matches!(
            agent_effect(methods::AGENT_DELETE, &json!({"agent_anchor": ANCHOR}), None),
            Plan::Refuse { .. }
        ));
    }
}
