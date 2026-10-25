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
    ObjectId, Source as _, methods,
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
    account.until("second identity page", |s, _| {
        s.participant(&"principal-bo".into()).is_some_and(|p| p.safety_number.is_some())
    });
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
fn an_oversized_row_fails_the_listing_instead_of_truncating_it() {
    let path = socket_path("budget");
    let fake = FakeBus::start(&path, |method, _, _| match method {
        "object.directory" => Reply::Error(
            "ValidationFailed",
            "local bus error: directory row exceeds response budget".to_owned(),
        ),
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("contexts", |s, _| s.is_listed(Listing::Contexts));
    account.settle(300);
    assert!(!account.store.is_listed(Listing::Objects), "objects are not claimed empty");
    assert!(account.store.object(&ObjectId::new("attachment:m1:0")).is_none());
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
    account.settle(200);
    assert!(
        !account.envelopes[before..]
            .iter()
            .any(|e| matches!(&e.event, DomainEvent::MomentAdded(m) if m.context_id.as_str() == "thread-team")),
        "nothing is shown as sent"
    );
    fake.shutdown();
}

#[test]
fn an_intervention_closes_with_the_decision_on_its_grant() {
    let path = socket_path("intervene");
    let fake = FakeBus::start(&path, |method, _, _| match method {
        "grant.approve" => Reply::Ok(json!({ "approval_id": "appr-1", "grant_id": "grant-1" })),
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    let moment = MomentId::new("approval:appr-1");
    account.until("intervention", |s, _| s.moment(&moment).is_some());
    account.source.submit(call(
        methods::GRANT_APPROVE,
        json!({ "grant_id": "appr-1", "args_hash": "h-123", "proposed_by": "Human",
            "confirmed_by": { "gesture": "Hold", "held_ms": 1200 } }),
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
    account.until("subscribed", |_, _| fake.subscribed("acct-main"));
    fake.push("acct-main", "mcp.approval.request", json!({
        "approval_id": "appr-9", "server_id": "files", "tool_name": "delete", "risk_level": "high",
        "expires_at": 1890000000, "status": "pending", "details": { "arguments_hash": "h-9" },
        "conversation_id": "thread-team", "context_id": "ctx-9",
    }));
    account.until("placed", |s, _| {
        s.moment(&MomentId::new("approval:appr-9"))
            .is_some_and(|m| m.context_id.as_str() == "thread-team")
    });
    fake.push("acct-main", "mcp.approval.denied", json!({ "approval_id": "appr-9" }));
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
fn a_saved_object_is_written_where_chosen_and_its_bytes_go_nowhere_else() {
    let path = socket_path("save");
    let fake = FakeBus::start(&path, |method, _, body| match method {
        "object.get" => Reply::Ok(json!({
            "object_id": body["object_id"],
            "plaintext_base64": ramflux_protocol::encode_base64url(b"secret notes"),
        })),
        _ => fixture(method),
    });
    let dir = std::env::temp_dir().join(format!("rf-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let target = dir.join("notes.txt");
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    let object = ObjectId::new("attachment:m1:0");
    account.until("object", |s, _| s.object(&object).is_some());
    account.source.submit(call(
        methods::OBJECT_SAVE,
        json!({ "object_id": "attachment:m1:0", "path": target.to_string_lossy() }),
    ));
    account.until("saved", |s, _| s.object(&object).is_some_and(|o| o.saved_to.is_some()));
    assert_eq!(std::fs::read(&target).expect("written"), b"secret notes");
    let get = fake.seen().into_iter().find(|s| s.method == "object.get").expect("get");
    assert!(get.operation_id.is_none(), "no receipt keeps the bytes");
    assert!(
        account.envelopes.iter().all(|e| !format!("{:?}", e.event).contains("secret notes")
            && !format!("{:?}", e.event)
                .contains(&ramflux_protocol::encode_base64url(b"secret notes"))),
        "the bytes never become an event"
    );
    let _ = std::fs::remove_dir_all(dir);
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
fn verifying_a_contact_reads_the_identity_directory_again() {
    let path = socket_path("verify");
    let verified = Arc::new(AtomicUsize::new(0));
    let flag = verified.clone();
    let fake = FakeBus::start(&path, move |method, _, _| match method {
        "contact.verify" => {
            flag.store(1, Ordering::SeqCst);
            Reply::Ok(
                json!({ "contact_identity_commitment": "cm-ana", "verification_state": "verified",
                "safety_number": "11111 22222" }),
            )
        }
        "identity.directory" if flag.load(Ordering::SeqCst) == 1 => {
            let mut frame = frames()[method].clone();
            frame["identities"][0]["safety_state"] = json!("verified");
            frame["identities"][0]["safety_number"] = json!("11111 22222");
            Reply::Ok(frame)
        }
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("ana", |s, _| s.participant(&"principal-ana".into()).is_some());
    account
        .source
        .submit(call(methods::SAFETY_VERIFY, json!({ "participant_id": "principal-ana" })));
    account.until("verified", |s, _| {
        s.participant(&"principal-ana".into()).is_some_and(|p| {
            p.safety == ramflux_desktop_core::SafetyState::Verified
                && p.safety_number.as_deref() == Some("11111 22222")
        })
    });
    let verify = fake.seen().into_iter().find(|s| s.method == "contact.verify").expect("verify");
    assert_eq!(verify.body, json!({ "contact_identity_commitment": "cm-ana" }));
    fake.shutdown();
}
