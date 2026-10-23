// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The request path against a fake daemon that, like the real one, answers
//! the requests of a connection concurrently and keeps retry receipts: one
//! multiplexed connection per scope, a deadline on every frame, a timeout
//! instead of a hang, a lost answer resent under the same key and applied
//! once, and a graceful daemon stop.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use common::*;
use ramflux_desktop_core::{
    AccountId, ContextId, Effect, FailureCode, GrantId, GrantState, NodeId, Source as _,
    SourceStatus, methods,
};
use ramflux_desktop_sdk::{Deadlines, SdkConfig, SdkHub};
use serde_json::json;

fn now_ms() -> u64 {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("clock");
    u64::try_from(now.as_millis()).expect("millis")
}

#[test]
fn the_reads_of_a_scope_run_concurrently_on_one_connection() {
    let path = socket_path("mux");
    let fake = FakeBus::start(&path, |method, _, _| match method {
        "conversation.directory" | "group.list" | "contact.list" | "mcp.approval.list" => {
            Reply::Slow(Duration::from_millis(700), frames()[method].clone())
        }
        _ => fixture(method),
    });
    let hub = hub(&path);
    let started = Instant::now();
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("snapshot", |s, _| {
        s.context(&ContextId::new("dm-ana")).is_some()
            && s.context(&ContextId::new("thread-team")).is_some()
    });
    let took = started.elapsed();
    assert!(took < Duration::from_millis(1_800), "four slow reads overlapped: {took:?}");
    let seen = fake.seen();
    let requests: Vec<_> = seen
        .iter()
        .filter(|s| s.account.as_deref() == Some("acct-main") && s.method != "subscription.open")
        .collect();
    assert!(requests.len() >= 10, "the whole snapshot: {}", requests.len());
    let connection = requests[0].connection;
    assert!(
        requests.iter().all(|s| s.connection == connection),
        "every request of the scope uses one connection"
    );
    let mut ids: Vec<_> = requests.iter().map(|s| s.request_id.clone()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), requests.len(), "each request has its own id");
    fake.shutdown();
}

#[test]
fn every_frame_carries_a_deadline_and_only_mutations_carry_a_key() {
    let path = socket_path("deadline");
    let fake = FakeBus::start(&path, |method, _, _| fixture(method));
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("approval", |s, _| s.grant(&GrantId::new("appr-1")).is_some());
    let before = now_ms();
    account.source.submit(Effect::Call {
        method: methods::GRANT_DENY.to_owned(),
        body: json!({ "grant_id": "appr-1" }),
    });
    account.until("denied", |s, _| {
        s.grant(&GrantId::new("appr-1")).is_some_and(|g| g.state == GrantState::Denied)
    });
    let seen = fake.seen();
    // The test configuration gives every method three seconds.
    for frame in &seen {
        let deadline =
            frame.deadline_ms.unwrap_or_else(|| panic!("{} has no deadline", frame.method));
        assert!(
            deadline > before.saturating_sub(10_000) && deadline < now_ms() + 3_100,
            "{frame:?}"
        );
    }
    let deny = seen.iter().find(|s| s.method == "grant.deny").expect("deny");
    assert!(deny.deadline_ms.is_some_and(|d| d >= before + 2_900), "budget from sending");
    assert!(deny.operation_id.as_deref().is_some_and(|k| k.starts_with("desk-op-")));
    assert!(
        seen.iter().filter(|s| s.method != "grant.deny").all(|s| s.operation_id.is_none()),
        "reads and pulls carry no key"
    );
    fake.shutdown();
}

#[test]
fn a_request_past_its_deadline_fails_as_a_timeout_and_never_hangs() {
    let path = socket_path("timeout");
    let fake = FakeBus::start(&path, |method, _, _| match method {
        "node.probe" => Reply::Slow(Duration::from_secs(4), frames()["node.probe"].clone()),
        _ => fixture(method),
    });
    let config =
        SdkConfig { deadlines: Deadlines::uniform(Duration::from_millis(800)), ..config(&path) };
    let hub = SdkHub::start(config).expect("bridge");
    let mut daemon = Fed::new(hub.daemon_source());
    daemon.until("node", |s, _| s.node(&NodeId::new("n-harbor")).is_some());
    let sent = Instant::now();
    daemon.source.submit(Effect::Call {
        method: methods::NODE_PROBE.to_owned(),
        body: json!({ "node_id": "n-harbor" }),
    });
    daemon.until("timed out", |s, _| !s.failures().is_empty());
    let waited = sent.elapsed();
    assert!(waited < Duration::from_millis(2_000), "ended at the deadline: {waited:?}");
    let failure = daemon.store.failures()[0].clone();
    assert_eq!(
        (failure.method.as_str(), &failure.code),
        (methods::NODE_PROBE, &FailureCode::Timeout)
    );
    assert_eq!(failure.message, "backend.failure.timeout", "a translated timeout state");
    let probes = fake.seen().iter().filter(|s| s.method == "node.probe").count();
    assert_eq!(probes, 1, "a probe has no key and is not resent");
    // The connection stays usable: polls keep being answered.
    let polled = fake.seen().len();
    daemon.settle(400);
    assert!(fake.seen().len() > polled);
    assert_eq!(daemon.store.source_status(), SourceStatus::Connected);
    fake.shutdown();
}

#[test]
fn a_lost_answer_is_resent_under_the_same_key_and_applied_once() {
    let path = socket_path("replay");
    let approvals = Arc::new(AtomicUsize::new(0));
    let counter = approvals.clone();
    let fake = FakeBus::start(&path, move |method, _, _| match method {
        "grant.approve" => {
            // The first time the daemon applies it, then the connection
            // breaks before the answer is written.
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                Reply::Lost(json!({ "approval_id": "appr-1", "status": "granted" }))
            } else {
                Reply::Ok(json!({ "approval_id": "appr-1", "status": "granted" }))
            }
        }
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("approval", |s, _| s.grant(&GrantId::new("appr-1")).is_some());
    account.source.submit(Effect::Call {
        method: methods::GRANT_APPROVE.to_owned(),
        body: json!({ "grant_id": "appr-1", "args_hash": "h", "proposed_by": "Human",
            "confirmed_by": { "gesture": "Hold", "held_ms": 1200 } }),
    });
    account.until("approved", |s, _| {
        s.grant(&GrantId::new("appr-1")).is_some_and(|g| g.state == GrantState::Approved)
    });
    let frames: Vec<_> = fake.seen().into_iter().filter(|s| s.method == "grant.approve").collect();
    assert_eq!(frames.len(), 2, "sent, answer lost, resent once");
    assert_eq!(frames[0].operation_id, frames[1].operation_id, "under the same key");
    assert_ne!(frames[0].connection, frames[1].connection, "on a new connection");
    assert!(!frames[0].replayed && frames[1].replayed, "the daemon answered from its receipt");
    assert_eq!(approvals.load(Ordering::SeqCst), 1, "applied once");
    assert!(account.store.failures().is_empty(), "{:?}", account.store.failures());
    fake.shutdown();
}

#[test]
fn a_missed_deadline_on_a_keyed_request_is_resent_and_answered_from_the_receipt() {
    let path = socket_path("late");
    let denials = Arc::new(AtomicUsize::new(0));
    let counter = denials.clone();
    let fake = FakeBus::start(&path, move |method, _, _| match method {
        "grant.deny" => {
            counter.fetch_add(1, Ordering::SeqCst);
            Reply::Slow(
                Duration::from_millis(1_500),
                json!({ "approval_id": "appr-1", "status": "denied" }),
            )
        }
        _ => fixture(method),
    });
    let config =
        SdkConfig { deadlines: Deadlines::uniform(Duration::from_millis(800)), ..config(&path) };
    let hub = SdkHub::start(config).expect("bridge");
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("approval", |s, _| s.grant(&GrantId::new("appr-1")).is_some());
    account.source.submit(Effect::Call {
        method: methods::GRANT_DENY.to_owned(),
        body: json!({ "grant_id": "appr-1" }),
    });
    account.until("denied", |s, _| {
        s.grant(&GrantId::new("appr-1")).is_some_and(|g| g.state == GrantState::Denied)
    });
    let denies: Vec<_> = fake.seen().into_iter().filter(|s| s.method == "grant.deny").collect();
    assert_eq!(denies.len(), 2);
    assert_eq!(denies[0].operation_id, denies[1].operation_id);
    assert!(denies[1].replayed, "the second answer came from the receipt");
    assert_eq!(denials.load(Ordering::SeqCst), 1, "applied once");
    assert!(account.store.failures().is_empty(), "{:?}", account.store.failures());
    fake.shutdown();
}

#[test]
fn the_hub_stops_the_daemon_gracefully_and_reports_a_refusal() {
    let path = socket_path("stop");
    let fake = FakeBus::start(&path, |method, _, _| match method {
        "daemon.stop" => Reply::Ok(json!({ "stopping": true })),
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut stop = hub.stop_daemon();
    assert_eq!(stop.wait(Duration::from_secs(3)), Some(Ok(())));
    let frame = fake.seen().into_iter().find(|s| s.method == "daemon.stop").expect("stop frame");
    assert!(frame.deadline_ms.is_some() && frame.operation_id.is_none());
    assert_eq!(frame.account, None);
    fake.shutdown();

    // Nothing listens: the stop fails with a reason, at once.
    let gone = SdkHub::start(config(&socket_path("stop-gone"))).expect("bridge");
    let mut stop = gone.stop_daemon();
    let answer = stop.wait(Duration::from_secs(3)).expect("an answer");
    assert!(answer.is_err(), "{answer:?}");
}
