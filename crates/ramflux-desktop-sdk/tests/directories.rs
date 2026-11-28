// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The client read model against a fake daemon: paged directories (short
//! pages under the response budget, an oversized row), the group send that
//! refuses unresolved member routes, approvals in their conversation, and
//! the actions the directories unlock (direct send, object save, member add
//! on several devices, verification).

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::*;
use ramflux_desktop_core::{
    AccountId, ContextId, DomainEvent, Effect, FailureCode, GrantId, Lifecycle, Listing, MomentId,
    Receipt, Source as _, methods,
};
use serde_json::{Value, json};

fn call(method: &str, body: Value) -> Effect {
    Effect::Call { method: method.to_owned(), body }
}

fn rows(method: &str, field: &str, ids: &[&str]) -> Vec<Value> {
    frames()[method][field]
        .as_array()
        .expect("rows")
        .iter()
        .filter(|row| ids.contains(&row["id"].as_str().unwrap_or_default()))
        .cloned()
        .collect()
}

#[test]
fn pages_are_followed_by_cursor_and_listed_only_when_complete() {
    let path = socket_path("pages");
    let fake = FakeBus::start(&path, |method, _, body| match (method, body["cursor"].as_str()) {
        ("conversation.directory", None) => Reply::Ok(json!({
            "conversations": rows(method, "conversations", &["dm-ana"]), "next_cursor": "c-1",
        })),
        // A page cut short by the response budget still has a next page.
        ("conversation.directory", Some("c-1")) => {
            Reply::Ok(json!({ "conversations": [], "next_cursor": "c-2" }))
        }
        ("conversation.directory", Some("c-2")) => Reply::Ok(json!({
            "conversations": rows(method, "conversations", &["thread-team"]), "next_cursor": null,
        })),
        ("identity.directory", None) => Reply::Ok(json!({
            "identities": rows(method, "identities", &["principal-ana", "principal-me"]),
            "next_cursor": "i-1",
        })),
        ("identity.directory", Some("i-1")) => Reply::Ok(json!({
            "identities": rows(method, "identities", &["principal-bo", "principal-cy"]),
            "next_cursor": null,
        })),
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("both pages of contexts", |s, _| {
        s.is_listed(Listing::Contexts)
            && s.context(&ContextId::new("dm-ana")).is_some()
            && s.context(&ContextId::new("thread-team")).is_some()
    });
    account.until("second identity page", |s, _| s.participant(&"principal-bo".into()).is_some());
    let seen = fake.seen();
    let cursors: Vec<Option<&str>> = seen
        .iter()
        .filter(|s| s.method == "conversation.directory")
        .map(|s| s.body["cursor"].as_str())
        .collect();
    // Polls may start new listings in between; each cursor follows a first
    // page and every next page was asked for.
    let first = cursors.iter().position(Option::is_none).expect("a first page");
    let c1 = cursors.iter().position(|c| *c == Some("c-1")).expect("page 2");
    let c2 = cursors.iter().position(|c| *c == Some("c-2")).expect("page 3");
    assert!(first < c1 && c1 < c2, "{cursors:?}");
    assert!(
        seen.iter()
            .filter(|s| s.method.ends_with(".directory"))
            .all(|s| s.body["limit"] == 100 && s.account.as_deref() == Some("acct-main"))
    );
    // A context was listed only once its listing was complete: nothing was
    // removed and re-added on the way.
    assert!(!account.names().iter().any(|n| n == "ContextRemoved"), "{:?}", account.names());
    assert!(account.store.rejections().is_empty(), "{:?}", account.store.rejections());
    fake.shutdown();
}

#[test]
fn a_group_send_with_unresolved_members_fails_and_names_them() {
    let path = socket_path("routes");
    let fake = FakeBus::start(&path, |method, _, _| {
        match method {
        "group.send" => Reply::Error(
            "ValidationFailed",
            r#"local bus error: group routes unresolved: {"group_id":"g-team","unreachable_members":["device-bo","device-zz"]}"#
                .to_owned(),
        ),
        _ => fixture(method),
    }
    });
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("identities and groups", |s, _| {
        s.context(&ContextId::new("thread-team")).is_some()
            && s.participant(&"principal-bo".into()).is_some()
    });
    let before = account.envelopes.len();
    account.source.submit(call(
        methods::SEND_MESSAGE,
        json!({ "context_id": "thread-team", "text": "hello all" }),
    ));
    account.until("failed", |s, _| !s.failures().is_empty());
    let failure = account.store.failures()[0].clone();
    assert_eq!(failure.method, methods::SEND_MESSAGE);
    assert_eq!(failure.code, FailureCode::Daemon("ValidationFailed".to_owned()));
    assert_eq!(
        failure.message,
        r#"backend.failure.group_routes_unresolved {"members":"principal-bo (device-bo), device-zz"}"#,
        "every unreachable member, by name when known"
    );
    let send = fake.seen().into_iter().find(|s| s.method == "group.send").expect("send");
    assert_eq!(
        (send.body["group_id"].as_str(), send.body["conversation_id"].as_str()),
        (Some("g-team"), Some("thread-team"))
    );
    assert!(send.body["target_delivery_id"].is_null(), "the daemon resolves every route");
    // The optimistic row is Sending before the refusal, then terminal Failed.
    // It must never look accepted or be scheduled as a networking wait.
    let failed: Vec<_> = account
        .store
        .moments()
        .filter(|m| {
            m.context_id.as_str() == "thread-team" && account.store.body(&m.id) == Some("hello all")
        })
        .collect();
    assert_eq!(failed.len(), 1, "one optimistic message retains its identity");
    let failed = failed[0];
    assert_eq!(failed.lifecycle, Lifecycle::Failed);
    assert!(account.envelopes[before..].iter().any(|e| matches!(&e.event,
        DomainEvent::MomentAdded(m) if m.id == failed.id
            && matches!(&m.kind, ramflux_desktop_core::MomentKind::Message(p) if p.receipt == Receipt::Sending))),
        "the initial row is Sending, not an accepted message");
    assert!(matches!(&failed.kind, ramflux_desktop_core::MomentKind::Message(p)
        if p.receipt == Receipt::Failed { retryable: false }));
    for event in &account.envelopes[before..] {
        let payload = match &event.event {
            DomainEvent::MomentAdded(m) if m.id == failed.id => Some(&m.kind),
            DomainEvent::MomentUpdated { moment_id, kind, .. } if *moment_id == failed.id => {
                kind.as_ref()
            }
            _ => None,
        };
        if let Some(ramflux_desktop_core::MomentKind::Message(p)) = payload {
            assert!(
                matches!(p.receipt, Receipt::Sending | Receipt::Failed { retryable: false }),
                "an unresolved route is never sent or waiting: {:?}",
                p.receipt
            );
        }
    }
    assert_eq!(fake.seen().iter().filter(|s| s.method == "group.send").count(), 1);
    fake.shutdown();
}

#[test]
fn an_intervention_closes_with_the_decision_on_its_grant() {
    let path = socket_path("intervene");
    let approvals = Arc::new(std::sync::Mutex::new(frames()["mcp.approval.list"].clone()));
    let listed = approvals.clone();
    let fake = FakeBus::start(&path, move |method, _, _| match method {
        "mcp.approval.list" => Reply::Ok(listed.lock().expect("approvals").clone()),
        "grant.approve" => Reply::Ok(json!({ "approval_id": "appr-1", "grant_id": "grant-1" })),
        _ => fixture(method),
    });
    fake.enable_stream();
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    let moment = MomentId::new("approval:appr-1");
    account.until("intervention", |s, _| s.moment(&moment).is_some());
    account.source.submit(call(
        methods::GRANT_APPROVE,
        json!({ "grant_id": "appr-1", "args_hash": "h-123", "proposed_by": "Human",
            "confirmed_by": { "gesture": "Tap" } }),
    ));
    account.until("done", |s, _| s.moment(&moment).is_some_and(|m| m.lifecycle == Lifecycle::Done));
    assert!(
        !ramflux_desktop_core::selectors::attention(&account.store)
            .iter()
            .any(|a| a.moment_id == moment),
        "no longer needs me"
    );
    // A pushed request of another conversation lands there. The fake
    // pushes only to open subscriptions: wait for the hub's to reach it.
    account.until("subscribed", |_, _| fake.streaming("acct-main"));
    let request = json!({
        "approval_id": "appr-9", "server_id": "files", "tool_name": "delete", "risk_level": "high",
        "expires_at": 1890000000, "status": "pending", "details": { "arguments_hash": "h-9" },
        "conversation_id": "thread-team", "context_id": "ctx-9",
    });
    approvals.lock().expect("approvals")["approvals"]
        .as_array_mut()
        .expect("rows")
        .push(request.clone());
    fake.publish("acct-main", "mcp.approval.request", request);
    account.until("placed", |s, _| {
        s.moment(&MomentId::new("approval:appr-9"))
            .is_some_and(|m| m.context_id.as_str() == "thread-team")
    });
    for row in approvals.lock().expect("approvals")["approvals"].as_array_mut().expect("rows") {
        if row["approval_id"] == "appr-9" {
            row["status"] = json!("denied");
        }
    }
    fake.publish("acct-main", "mcp.approval.denied", json!({ "approval_id": "appr-9" }));
    account.until("dismissed", |s, _| {
        s.moment(&MomentId::new("approval:appr-9"))
            .is_some_and(|m| m.lifecycle == Lifecycle::Dismissed)
    });
    assert!(account.store.grant(&GrantId::new("appr-9")).is_some());
    fake.shutdown();
}

#[test]
fn a_direct_message_goes_out_over_the_listed_route() {
    let path = socket_path("direct");
    let fake = FakeBus::start(&path, |method, _, body| match method {
        "message.submit" => {
            Reply::Ok(json!({ "message_id": body["message_id"], "accepted": true }))
        }
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("contexts", |s, _| s.is_listed(Listing::Contexts));
    account.settle(100);
    account
        .source
        .submit(call(methods::SEND_MESSAGE, json!({ "context_id": "dm-ana", "text": "hi" })));
    account.until("submitted", |_, _| fake.seen().iter().any(|s| s.method == "message.submit"));
    let submit = fake.seen().into_iter().find(|s| s.method == "message.submit").expect("submit");
    assert_eq!(submit.body["recipient_device_id"], "device-ana");
    assert_eq!(submit.body["target_delivery_id"], "target-ana");
    assert_eq!(submit.body["recipient_principal_commitment"], "cm-ana");
    assert_eq!(submit.body["source_principal_id"], "principal-me");
    assert!(submit.operation_id.is_some());
    account.settle(200);
    assert!(account.store.failures().is_empty(), "{:?}", account.store.failures());
    fake.shutdown();
}

#[test]
fn adding_a_member_on_several_devices_reports_how_far_it_got() {
    let path = socket_path("adds");
    let adds = Arc::new(AtomicUsize::new(0));
    let counter = adds.clone();
    let fake = FakeBus::start(&path, move |method, _, _| match method {
        "group.member.add" if counter.fetch_add(1, Ordering::SeqCst) == 0 => {
            Reply::Ok(json!({ "group_id": "g-team" }))
        }
        "group.member.add" => {
            Reply::Error("ValidationFailed", "local bus error: member limit".to_owned())
        }
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("identities", |s, _| {
        s.context(&ContextId::new("thread-team")).is_some()
            && s.participant(&"principal-cy".into()).is_some()
    });
    account.source.submit(call(
        methods::GROUP_ADD_MEMBER,
        json!({ "context_id": "thread-team", "participant_id": "principal-cy", "role": "Member" }),
    ));
    account.until("failed", |s, _| !s.failures().is_empty());
    let failure = account.store.failures()[0].clone();
    let (key, args) = failure.message.split_once(' ').expect("a key with arguments");
    assert_eq!(key, "backend.failure.partial");
    assert_eq!(
        serde_json::from_str::<Value>(args).expect("arguments"),
        json!({ "done": "1", "total": "2", "reason": "local bus error: member limit" })
    );
    let sent: Vec<Value> = fake
        .seen()
        .into_iter()
        .filter(|s| s.method == "group.member.add")
        .map(|s| s.body)
        .collect();
    assert_eq!(sent.len(), 2, "stopped at the failure");
    assert_eq!(
        (sent[0]["member_id"].as_str(), sent[1]["member_id"].as_str()),
        (Some("device-cy-1"), Some("device-cy-2"))
    );
    fake.shutdown();
}

#[test]
fn accepting_an_identity_change_names_the_anchor_shown_and_reads_the_status_again() {
    let path = socket_path("accept");
    let fake = FakeBus::start(&path, move |method, _, body| match method {
        "contact.trust.accept_change" if body["new_anchor"] == "A2" => {
            Reply::Ok(json!({ "state": "accepted" }))
        }
        "contact.verification.status" => Reply::Ok(json!({ "identity_state": "tofu" })),
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("ana", |s, _| s.participant(&"principal-ana".into()).is_some());
    account.source.submit(call(
        methods::IDENTITY_ACCEPT_CHANGE,
        json!({ "participant_id": "principal-ana", "new_anchor": "A2" }),
    ));
    account.until("the status read again", |_, _| {
        fake.seen().iter().any(|s| s.method == "contact.verification.status")
    });
    let accept = fake
        .seen()
        .into_iter()
        .find(|s| s.method == "contact.trust.accept_change")
        .expect("accept_change");
    assert_eq!(accept.body, json!({ "contact": "cm-ana", "new_anchor": "A2" }));
    fake.shutdown();
}
