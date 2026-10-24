// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Starting a direct conversation: the first message goes out as
//! `message.submit` under a fresh conversation id, to the route the identity
//! directory lists for the contact (its commitment and the first device with
//! a verified key and a delivery target). Without such a device the start is
//! refused, naming what is missing; nothing is guessed. Against a fake
//! daemon the conversation is shown once the submission was accepted, with
//! its first message.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use common::*;
use ramflux_desktop_core::{
    AccountId, ContextId, ContextKind, DomainEvent, Effect, FailureCode, MomentId, Source as _,
    methods,
};
use ramflux_desktop_sdk::bus::BusRequest;
use ramflux_desktop_sdk::directory::Directory;
use ramflux_desktop_sdk::plan::fetch::{Fetch, Paged};
use ramflux_desktop_sdk::plan::{Known, Plan, plan};
use ramflux_desktop_sdk::translate::reason::key;
use ramflux_sdk::LocalBusMessageSubmitRequest;
use serde_json::{Value, json};

struct Listed(Directory);

impl Known for Listed {
    fn context_kind(&self, context: &ContextId) -> Option<ContextKind> {
        (context.as_str() == "dm-ana").then_some(ContextKind::Direct)
    }
    fn self_device(&self) -> Option<&str> {
        Some("device-me")
    }
    fn daemon_grant_id(&self, _: &str) -> Option<&str> {
        None
    }
    fn directory(&self) -> Option<&Directory> {
        Some(&self.0)
    }
}

fn start_body(context: &str, participant: &str) -> Value {
    json!({
        "context_id": context,
        "participant_id": participant,
        "text": "Hello Ana",
        "title": "Ana",
        "proposed_by": "Human",
        "confirmed_by": null,
    })
}

fn planned_with(directory: Directory, body: Value) -> Plan {
    let effect = Effect::Call { method: methods::CONVERSATION_START.to_owned(), body };
    plan(Some(&AccountId::new("acct-main")), effect, &Listed(directory)).plan
}

fn refusal(plan: Plan) -> String {
    match plan {
        Plan::Refuse { code: FailureCode::UnsupportedByBackend, message } => message,
        other => panic!("a refusal with a reason expected, got {other:?}"),
    }
}

const NEW: &str = "0f3c2a91-5b7e-4d0a-9c1e-2f4b6d8a0c13";

#[test]
fn the_first_message_goes_to_the_listed_identity_route() {
    let Plan::Call { request, refresh } =
        planned_with(common::directory(), start_body(NEW, "principal-ana"))
    else {
        panic!("one request expected");
    };
    let request: BusRequest = request;
    assert_eq!(request.method, "message.submit");
    assert!(request.operation_id.is_some(), "a start carries a retry key");
    let body = request.wire_body();
    assert_eq!(body["title"], "Ana", "the contact's name is the local title");
    let submit: LocalBusMessageSubmitRequest =
        serde_json::from_value(body.clone()).expect("the SDK's request type");
    assert_eq!(submit.conversation_id, NEW);
    assert_eq!(submit.source_principal_id, "principal-me");
    assert_eq!(submit.sender_id, "device-me");
    assert_eq!(submit.recipient_device_id.as_deref(), Some("device-ana"));
    assert_eq!(submit.recipient_principal_commitment.as_deref(), Some("cm-ana"));
    assert_eq!(submit.target_delivery_id, "target-ana");
    assert_eq!(
        submit.plaintext_body_base64,
        Some(ramflux_protocol::encode_base64url(b"Hello Ana"))
    );
    assert!(submit.federation.is_none() && submit.attachments.is_empty());
    assert_eq!(refresh, [Fetch::first(Paged::Conversations), Fetch::Messages(ContextId::new(NEW))]);
}

#[test]
fn the_first_verified_device_with_a_target_is_chosen() {
    let mut directory = common::directory();
    // Cy has two devices; the first loses its target.
    let cy = directory.identities.get_mut("principal-cy").expect("cy");
    cy.devices[0].target = None;
    let Plan::Call { request, .. } = planned_with(directory, start_body(NEW, "principal-cy"))
    else {
        panic!("one request expected");
    };
    assert_eq!(request.wire_body()["recipient_device_id"], "device-cy-2");
    assert_eq!(request.wire_body()["target_delivery_id"], "target-cy-2");
}

#[test]
fn a_missing_route_is_refused_with_what_is_missing() {
    let named = |reason: &str| format!("{reason} {{\"name\":\"Ana\"}}");
    let unknown = refusal(planned_with(common::directory(), start_body(NEW, "principal-zed")));
    assert_eq!(unknown, named(key::START_NO_IDENTITY));

    let mut no_target = common::directory();
    no_target.identities.get_mut("principal-ana").expect("ana").devices[0].target = None;
    let reason = refusal(planned_with(no_target, start_body(NEW, "principal-ana")));
    assert_eq!(reason, named(key::START_NO_TARGET));

    let mut unverified = common::directory();
    unverified.identities.get_mut("principal-ana").expect("ana").devices[0].signing_key = None;
    let reason = refusal(planned_with(unverified, start_body(NEW, "principal-ana")));
    assert_eq!(reason, named(key::START_UNVERIFIED_KEY));

    let mut no_commitment = common::directory();
    no_commitment.identities.get_mut("principal-ana").expect("ana").commitment = None;
    let reason = refusal(planned_with(no_commitment, start_body(NEW, "principal-ana")));
    assert_eq!(reason, key::NO_COMMITMENT);
}

#[test]
fn a_known_conversation_is_never_started_again() {
    let plan = planned_with(common::directory(), start_body("dm-ana", "principal-ana"));
    assert!(matches!(plan, Plan::Refuse { .. }), "{plan:?}");
}

/// The fixture's conversation directory, plus the new conversation once
/// `started` is set (as the daemon records it from the submission).
fn conversations(started: bool) -> Value {
    let mut frame = frames()["conversation.directory"].clone();
    if started {
        let mut row = frame["conversations"][0].clone();
        row["id"] = json!(NEW);
        row["title"] = json!("Ana");
        row["unread_count"] = json!(0);
        row["is_unread"] = json!(false);
        frame["conversations"].as_array_mut().expect("rows").push(row);
    }
    frame
}

fn new_messages() -> Value {
    json!({ "messages": [{
        "conversation_id": NEW, "message_id": "desk-msg-first", "sender_id": "device-me",
        "encrypted_body": b"Hello Ana".to_vec(), "deleted": false, "created_at": 1_790_000_200,
        "receipts": [],
    }], "rejected": [] })
}

#[test]
fn a_started_conversation_is_shown_with_its_first_message() {
    let path = socket_path("startdm");
    let started = Arc::new(AtomicBool::new(false));
    let flag = started.clone();
    let fake = FakeBus::start(&path, move |method, _, body| match method {
        "message.submit" => {
            flag.store(true, Ordering::SeqCst);
            Reply::Ok(json!({ "message_id": "desk-msg-first", "envelope_id": "e" }))
        }
        "conversation.directory" => Reply::Ok(conversations(flag.load(Ordering::SeqCst))),
        "message.list" if body["conversation_id"] == NEW => Reply::Ok(new_messages()),
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("the identities", |s, _| {
        s.context(&ContextId::new("dm-ana")).is_some()
            && s.participant(&"principal-ana".into()).is_some()
    });
    account.source.submit(Effect::Call {
        method: methods::CONVERSATION_START.to_owned(),
        body: start_body(NEW, "principal-ana"),
    });
    let id = ContextId::new(NEW);
    account.until("the new conversation", |s, _| s.context(&id).is_some());
    let context = account.store.context(&id).cloned().expect("context");
    assert_eq!(context.kind, ContextKind::Direct);
    assert_eq!(context.title, "Ana");
    account.source.focus(Some(&id));
    let first = MomentId::new(format!("msg:{NEW}:desk-msg-first"));
    account.until("the first message", |s, _| s.body(&first).is_some());
    assert_eq!(account.store.body(&first), Some("Hello Ana"));
    account.settle(400);
    assert!(account.store.context(&id).is_some(), "the directory row keeps it");
    assert!(
        !account.envelopes.iter().any(|e| matches!(
            &e.event, DomainEvent::ContextRemoved { context_id } if *context_id == id
        )),
        "never removed while the directory caught up"
    );
    let submits: Vec<_> =
        fake.seen().into_iter().filter(|s| s.method == "message.submit").collect();
    assert_eq!(submits.len(), 1);
    assert_eq!(submits[0].body["conversation_id"], NEW);
    drop(account);
    fake.shutdown();
}

/// A listing the daemon read before the start can reach the source after a
/// newer one that already lists the conversation (requests share one
/// connection and are answered as they finish). That older listing is out
/// of date and must not remove what the newer one listed.
#[test]
fn a_listing_read_before_the_start_and_answered_last_removes_nothing() {
    let path = socket_path("startlate");
    let started = Arc::new(AtomicBool::new(false));
    let hold = Arc::new(AtomicBool::new(false));
    let (flag, held) = (started.clone(), hold.clone());
    let fake = FakeBus::start(&path, move |method, _, body| match method {
        "message.submit" => {
            flag.store(true, Ordering::SeqCst);
            Reply::Ok(json!({ "message_id": "desk-msg-first", "envelope_id": "e" }))
        }
        "conversation.directory" => {
            let listed = flag.load(Ordering::SeqCst);
            let frame = conversations(listed);
            // Read now, answered after the start and its own listing.
            if held.load(Ordering::SeqCst) && !listed {
                Reply::Slow(Duration::from_millis(600), frame)
            } else {
                Reply::Ok(frame)
            }
        }
        "message.list" if body["conversation_id"] == NEW => Reply::Ok(new_messages()),
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("the identities", |s, _| {
        s.context(&ContextId::new("dm-ana")).is_some()
            && s.participant(&"principal-ana".into()).is_some()
    });
    hold.store(true, Ordering::SeqCst);
    let mark = fake.seen().len();
    account.until("a listing read before the start", |_, _| {
        fake.seen().iter().skip(mark).any(|s| s.method == "conversation.directory")
    });
    account.source.submit(Effect::Call {
        method: methods::CONVERSATION_START.to_owned(),
        body: start_body(NEW, "principal-ana"),
    });
    let id = ContextId::new(NEW);
    account.until("a listing read after the start", |_, _| {
        let seen = fake.seen();
        seen.iter().position(|s| s.method == "message.submit").is_some_and(|submit| {
            seen[submit + 1..].iter().any(|s| s.method == "conversation.directory")
        })
    });
    account.until("the new conversation", |s, _| s.context(&id).is_some());
    // The held listing arrives now.
    account.settle(1_000);
    assert!(account.store.context(&id).is_some(), "still shown");
    assert!(
        !account.envelopes.iter().any(|e| matches!(
            &e.event, DomainEvent::ContextRemoved { context_id } if *context_id == id
        )),
        "an older listing answered last removes nothing"
    );
    drop(account);
    fake.shutdown();
}

#[test]
fn a_refused_start_reports_the_reason_and_creates_nothing() {
    let path = socket_path("startno");
    let fake = FakeBus::start(&path, |method, _, _| fixture(method));
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("the identities", |s, _| s.participant(&"principal-ana".into()).is_some());
    account.source.submit(Effect::Call {
        method: methods::CONVERSATION_START.to_owned(),
        body: start_body(NEW, "principal-zed"),
    });
    account.until("the refusal", |s, _| !s.failures().is_empty());
    let failure = account.store.failures().last().cloned().expect("failure");
    assert_eq!(failure.method, methods::CONVERSATION_START);
    assert!(failure.message.starts_with(key::START_NO_IDENTITY), "{}", failure.message);
    assert!(account.store.context(&ContextId::new(NEW)).is_none());
    assert!(!fake.seen().iter().any(|s| s.method == "message.submit"), "nothing was sent");
    drop(account);
    fake.shutdown();
}
