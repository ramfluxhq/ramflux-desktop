// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Named agents (AGENT-IDENTITY-01) at the SDK boundary: the body the daemon
//! receives for a creation and for an access change, the refusal a failed
//! creation reports, and the rule that the agent's PRX name comes from its
//! anchor. The fake daemon of the other suites does not speak these methods
//! yet; the live suite (`rfd_live`) covers the round trip.

use ramflux_desktop_core::{AccountId, AgentRefusal, Effect, methods};
use ramflux_desktop_sdk::bus::BusError;
use ramflux_desktop_sdk::plan::Plan;
use ramflux_desktop_sdk::plan::agents::{agent_effect, prx_name};
use ramflux_desktop_sdk::translate::agents::{failure_key, refusal_of};
use serde_json::json;

const ANCHOR: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

fn owner() -> AccountId {
    AccountId::new("acct-owner")
}

#[test]
fn a_creation_reaches_the_daemon_with_both_limits_and_no_host() {
    let effect = Effect::Call {
        method: methods::AGENT_CREATE.to_owned(),
        body: json!({
            "persona": {"name": "Ledger", "avatar": "preset:spark:teal", "responsibility": "Books"},
            "config": {
                "provider": "anthropic", "model": "m", "autonomy": "full", "tools": [],
                "limits": {"max_concurrent_turns": 1, "max_cost_per_day_cents": 100},
            },
            "access": {"dm": "owner", "group_inviters": "owner", "allowed_humans": []},
            "role": "named", "confirmed": true,
            "proposed_by": "Human", "confirmed_by": {"gesture": "Tap"},
        }),
    };
    let Plan::Call { request, .. } = agent_effect(
        effect.method().unwrap_or_default(),
        effect.body().unwrap_or(&json!(null)),
        Some(&owner()),
    ) else {
        panic!("test: a creation is a call")
    };
    assert_eq!(request.method, "agent.create");
    assert_eq!(request.body["confirmed"], true);
    assert_eq!(request.body["config"]["limits"]["max_cost_per_day_cents"], 100);
    assert!(request.body.get("host").is_none());
    assert!(request.body.get("proposed_by").is_none(), "audit stamps never reach the daemon");
}

#[test]
fn an_access_change_sends_the_record_base_it_was_read_with() {
    let base = json!({"id": "b-7", "hlc": 9});
    let body = json!({
        "agent_anchor": ANCHOR,
        "access": {"dm": "contacts", "group_inviters": "owner", "allowed_humans": []},
        "base": base,
    });
    let Plan::Call { request, .. } = agent_effect(methods::AGENT_ACCESS_SET, &body, Some(&owner()))
    else {
        panic!("test: an access change is a call")
    };
    assert_eq!(request.body["base"], base);
}

#[test]
fn a_job_names_its_agent_by_the_runtime_name_of_its_anchor() {
    let name = prx_name(ANCHOR).expect("test: a 32-byte anchor");
    assert!(name.starts_with("a_"));
    let body = json!({"agent_anchor": ANCHOR, "kind": "cron", "id": "j-1"});
    let Plan::Call { request, .. } = agent_effect(methods::AGENT_JOB_RESUME, &body, Some(&owner()))
    else {
        panic!("test: a job request is a call")
    };
    assert_eq!(request.body["agent"], name);
}

#[test]
fn a_refused_runtime_after_a_staged_creation_is_reported_as_created_without_runtime() {
    let error = BusError::Daemon {
        retry_after_ms: None,
        code: "prx_failed".into(),
        message: String::new(),
        reason_code: Some("prx_spawn".into()),
        details: None,
        cause: None,
    };
    assert_eq!(refusal_of(&error), AgentRefusal::PrxSpawn);
    assert_eq!(
        failure_key(methods::AGENT_CREATE, &error),
        Some("backend.agent.create.runtime_not_applied")
    );
}
