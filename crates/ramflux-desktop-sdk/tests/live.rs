// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Effects, error mapping, pushed events, polling and reconnects against a
//! fake daemon. The daemon reports no pins, so a conversation is followed
//! when the view focuses it.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use common::*;
use ramflux_desktop_core::{
    AccountId, ContextId, DomainEvent, Effect, FailureCode, GrantId, GrantState, MomentId,
    MomentKind, NodeId, Receipt, Source, SourceStatus, methods,
};
use serde_json::json;

fn call(method: &str, body: serde_json::Value) -> Effect {
    Effect::Call { method: method.to_owned(), body }
}

fn failures(fed: &Fed) -> Vec<(String, FailureCode)> {
    fed.store.failures().iter().map(|f| (f.method.clone(), f.code.clone())).collect()
}

#[test]
fn effects_reach_the_daemon_and_their_answers_become_events() {
    let path = socket_path("effects");
    let fake = FakeBus::start(&path, |method, _, body| match method {
        "grant.deny" => Reply::Error(
            "ValidationFailed",
            format!("local bus error: no approval {}", body["approval_id"]),
        ),
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("approval", |s, _| s.grant(&GrantId::new("appr-1")).is_some());
    account.source.submit(call(
        methods::GRANT_APPROVE,
        json!({ "grant_id": "appr-1", "args_hash": "h-123", "proposed_by": "Human", "confirmed_by": { "gesture": "Hold", "held_ms": 1200 } }),
    ));
    account.until("approved", |s, _| {
        s.grant(&GrantId::new("appr-1")).is_some_and(|g| g.state == GrantState::Approved)
    });
    assert!(
        account
            .store
            .audit()
            .iter()
            .any(|a| a.action == methods::GRANT_APPROVE && a.human_confirmed)
    );
    let approve =
        fake.seen().into_iter().find(|s| s.method == "grant.approve").expect("approve frame");
    assert_eq!(approve.account.as_deref(), Some("acct-main"));
    assert_eq!(approve.body, json!({ "approval_id": "appr-1" }));

    // A daemon error frame is a failed effect with the daemon's code.
    account.source.submit(call(methods::GRANT_DENY, json!({ "grant_id": "appr-9" })));
    account.until("deny failure", |s, _| !s.failures().is_empty());
    assert_eq!(
        failures(&account),
        [(methods::GRANT_DENY.to_owned(), FailureCode::Daemon("ValidationFailed".into()))]
    );
    assert!(account.store.failures()[0].message.contains("no approval"));

    // An unsupported method never reaches the daemon.
    let before = fake.seen().len();
    account.source.submit(call(methods::PRIVACY_REVEAL, json!({ "moment_id": "m1" })));
    account.until("unsupported", |s, _| s.failures().len() == 2);
    assert_eq!(account.store.failures()[1].code, FailureCode::UnsupportedByBackend);
    assert_eq!(account.store.failures()[1].message, "backend.gap.reveal");
    assert!(fake.seen()[before..].iter().all(|s| s.method != "privacy.reveal"));

    // A pin is a desktop-local preference.
    account.source.submit(call(methods::CONTEXT_PIN, json!({ "context_id": "thread-team" })));
    account.until("pinned", |s, _| {
        s.context(&ContextId::new("thread-team")).is_some_and(|c| c.pinned)
    });
    fake.shutdown();
}

#[test]
fn the_daemon_scope_creates_accounts_and_probes_nodes() {
    let path = socket_path("entry");
    let created = Arc::new(AtomicUsize::new(0));
    let seen = created.clone();
    let fake = FakeBus::start(&path, move |method, _, _| match method {
        // The daemon lists the new account, with the label it kept.
        "account.list" if seen.load(Ordering::SeqCst) > 0 => Reply::Ok(json!({
            "accounts": ["acct-new"], "active_account_id": "acct-new",
            "account_details": [{ "id": "acct-new", "label": "Personal", "kind": "human",
                "principal_id": "principal-new", "state": "unlocked" }],
        })),
        "account.list" => Reply::Ok(json!({ "accounts": [], "account_details": [] })),
        "account.create" => {
            seen.fetch_add(1, Ordering::SeqCst);
            fixture(method)
        }
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut daemon = Fed::new(hub.daemon_source());
    daemon.until("node", |s, _| s.node(&NodeId::new("n-harbor")).is_some());
    daemon.source.submit(Effect::SecretCall {
        method: methods::ACCOUNT_CREATE.to_owned(),
        body: json!({ "kind": "HumanInteractive", "label": "Personal", "node_id": "n-harbor" }),
        secrets: vec![ramflux_desktop_core::SecretInput::new("river stone lantern")],
    });
    daemon.until("account", |s, _| s.account(&AccountId::new("acct-new")).is_some());
    let account = daemon.store.account(&AccountId::new("acct-new")).cloned().expect("account");
    assert_eq!(account.label, "Personal", "the label the user typed");
    assert_eq!(account.lock_state, ramflux_desktop_core::LockState::Unlocked);
    let create = fake.seen().into_iter().find(|s| s.method == "account.create").expect("create");
    assert_eq!(create.body["account_secret"], "river stone lantern");
    assert_eq!(
        (create.body["label"].as_str(), create.body["kind"].as_str()),
        (Some("Personal"), Some("human"))
    );
    assert!(daemon.envelopes.iter().all(|e| !format!("{:?}", e.event).contains("river")));
    daemon.source.submit(call(methods::NODE_PROBE, json!({ "node_id": "n-harbor" })));
    daemon
        .until("probe", |s, _| s.node(&NodeId::new("n-harbor")).is_some_and(|n| n.probe.is_some()));
    let probe =
        daemon.store.node(&NodeId::new("n-harbor")).and_then(|n| n.probe.clone()).expect("probe");
    assert!(probe.gateway.ok && probe.relay.ok);
    assert_eq!(probe.gateway.latency_ms, Some(42));
    fake.shutdown();
}

#[test]
fn pushed_approvals_and_decisions_become_grant_events() {
    let path = socket_path("push");
    let fake = FakeBus::start(&path, |method, _, _| match method {
        "mcp.approval.list" => Reply::Ok(json!({ "approvals": [] })),
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("subscribed", |_, _| fake.subscribed("acct-main"));
    let open = fake.seen().into_iter().find(|s| s.method == "subscription.open").expect("open");
    assert_eq!(open.body["attended_frontend"], true);
    assert!(
        open.body["topics"].as_array().is_some_and(|t| t.contains(&json!("mcp.approval.request")))
    );
    fake.push("acct-main", "mcp.approval.request", json!({
        "event_type": "mcp.approval.request", "approval_id": "appr-7", "server_id": "mail",
        "tool_name": "send", "risk_level": "critical", "expires_at": 1890000000, "status": "pending",
        "details": { "arguments_hash": "h-7" },
    }));
    account.until("requested", |s, _| s.grant(&GrantId::new("appr-7")).is_some());
    assert_eq!(
        account.store.grant(&GrantId::new("appr-7")).map(|g| g.risk),
        Some(ramflux_desktop_core::Risk::Critical)
    );
    fake.push(
        "acct-main",
        "mcp.approval.denied",
        json!({ "approval_id": "appr-7", "status": "denied" }),
    );
    account.until("denied", |s, _| {
        s.grant(&GrantId::new("appr-7")).is_some_and(|g| g.state == GrantState::Denied)
    });
    fake.shutdown();
}

#[test]
fn polling_adds_each_message_once_and_updates_receipts() {
    let path = socket_path("poll");
    let polls = Arc::new(AtomicUsize::new(0));
    let counter = polls.clone();
    let fake = FakeBus::start(&path, move |method, _, _| match method {
        "message.list" => {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            let mut frame = frames()["message.list"].clone();
            if n >= 3 {
                frame["messages"][1]["receipts"][0]["state"] = json!("read");
            }
            Reply::Ok(frame)
        }
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.source.focus(Some(&ContextId::new("dm-ana")));
    account.until("several polls", |_, _| polls.load(Ordering::SeqCst) >= 5);
    account.settle(100);
    let added = account
        .envelopes
        .iter()
        .filter(|e| matches!(&e.event, DomainEvent::MomentAdded(m) if m.id == MomentId::new("msg:dm-ana:m2")))
        .count();
    assert_eq!(added, 1, "the same message is added once");
    let receipt =
        account.store.moment(&MomentId::new("msg:dm-ana:m2")).and_then(|m| match &m.kind {
            MomentKind::Message(p) => Some(p.receipt),
            _ => None,
        });
    assert_eq!(receipt, Some(Receipt::Read), "a changed receipt updates the moment");
    let receives = fake.seen().iter().filter(|s| s.method == "message.receive").count();
    assert!(receives >= 2, "the focused conversation is pulled from the node");
    let pull = fake.seen().into_iter().find(|s| s.method == "message.receive").expect("pull");
    assert_eq!(
        pull.body["auto_fetch_attachments"], false,
        "receiving never waits on an attachment download"
    );
    fake.shutdown();
}

#[test]
fn a_lost_daemon_reports_disconnected_and_a_new_one_is_resnapshotted() {
    let path = socket_path("reconnect");
    let fake = FakeBus::start(&path, |method, _, _| fixture(method));
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.source.focus(Some(&ContextId::new("dm-ana")));
    account.until("live", |s, _| s.moment(&MomentId::new("msg:dm-ana:m1")).is_some());
    fake.shutdown();
    account.until("disconnected", |s, _| s.source_status() == SourceStatus::Disconnected);
    let seq_at_loss = account.store.last_seq();
    let restarted = FakeBus::start(&path, |method, _, _| match method {
        "conversation.directory" => {
            let mut frame = frames()["conversation.directory"].clone();
            frame["conversations"]
                .as_array_mut()
                .expect("rows")
                .retain(|r| r["id"] != "thread-team");
            Reply::Ok(frame)
        }
        _ => fixture(method),
    });
    account.until("reconnected", |s, _| s.source_status() == SourceStatus::Connected);
    account.until("re-snapshot removes the gone conversation", |s, _| {
        s.context(&ContextId::new("thread-team")).is_none()
    });
    assert!(account.store.last_seq() > seq_at_loss, "the cursor keeps moving forward");
    assert_eq!(account.source.last_seq(), account.store.last_seq());
    assert!(account.store.rejections().is_empty(), "{:?}", account.store.rejections());
    restarted.shutdown();
}

#[test]
fn a_slow_request_of_one_account_does_not_block_another() {
    let path = socket_path("slow");
    // The slow account's directory is held until the test releases it.
    let release = Arc::new(AtomicUsize::new(0));
    let held = release.clone();
    let fake = FakeBus::start(&path, move |method, account, _| match (method, account) {
        ("conversation.directory", Some("acct-slow")) => {
            let limit = Instant::now() + Duration::from_secs(30);
            while held.load(Ordering::SeqCst) == 0 && Instant::now() < limit {
                std::thread::sleep(Duration::from_millis(5));
            }
            fixture(method)
        }
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut slow = Fed::new(hub.account_source(AccountId::new("acct-slow")));
    let mut fast = Fed::new(hub.account_source(AccountId::new("acct-main")));
    fast.until("fast account live", |s, _| s.context(&ContextId::new("dm-ana")).is_some());
    slow.pump();
    assert!(
        slow.store.context(&ContextId::new("dm-ana")).is_none(),
        "the slow account is still waiting while the other one is live"
    );
    release.store(1, Ordering::SeqCst);
    slow.until("slow account live", |s, _| s.context(&ContextId::new("dm-ana")).is_some());
    fake.shutdown();
}

#[test]
fn an_account_the_daemon_has_not_open_is_reported_locked() {
    let path = socket_path("locked");
    let recorded = frames()["account.status#recorded_error"].clone();
    let fake = FakeBus::start(&path, move |method, _, _| match method {
        "account.status" => Reply::Error(
            "ValidationFailed",
            recorded["message"].as_str().unwrap_or_default().to_owned(),
        ),
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-none")));
    account.until("locked", |s, _| {
        s.account(&AccountId::new("acct-none"))
            .is_some_and(|a| a.lock_state == ramflux_desktop_core::LockState::Locked)
    });
    account.settle(300);
    assert!(!fake.subscribed("acct-none"), "a locked account is not followed");
    fake.shutdown();
}

#[test]
fn an_evicted_body_is_read_again_when_the_view_asks() {
    let path = socket_path("bodies");
    let fake = FakeBus::start(&path, |method, _, _| fixture(method));
    let hub = hub(&path);
    let mut account = Fed {
        source: hub.account_source(AccountId::new("acct-main")),
        store: ramflux_desktop_core::Store::with_body_capacity(1),
        envelopes: Vec::new(),
    };
    account.source.focus(Some(&ContextId::new("dm-ana")));
    let first = MomentId::new("msg:dm-ana:m1");
    let second = MomentId::new("msg:dm-ana:m2");
    account
        .until("both messages", |s, _| s.moment(&first).is_some() && s.moment(&second).is_some());
    account.settle(100);
    let missing = if account.store.body(&first).is_none() { first } else { second };
    assert!(account.store.body(&missing).is_none(), "one body was evicted");
    account.source.request_bodies(std::slice::from_ref(&missing));
    account.until("the body is back", |s, _| s.body(&missing).is_some());
    assert!(account.store.rejections().is_empty(), "{:?}", account.store.rejections());
    fake.shutdown();
}
