// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! An attachment on its way: while its `message.submit` runs, the
//! conversation shows the message at once (receipt "sending", in progress)
//! with the object as its attachment, transferring (no bytes claimed); once
//! the daemon accepted it the message is done, and the listed message
//! updates the same moment. A refused send marks both failed with the
//! reason. A file over the frame budget is refused before anything is sent.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use ramflux_desktop_core::{
    AccountId, ContextId, Direction, Effect, Lifecycle, MessagePayload, Moment, MomentId,
    MomentKind, ObjectId, Receipt, Source as _, Store, TransferState, methods,
};
use serde_json::{Value, json};

fn attach(path: &std::path::Path) -> Effect {
    Effect::Call {
        method: methods::ATTACH_OBJECT.to_owned(),
        body: json!({ "context_id": "dm-ana", "path": path.display().to_string(),
                      "proposed_by": "Human", "confirmed_by": null }),
    }
}

fn file(name: &str, len: usize) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rf-attach-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join(format!("{name}.txt"));
    std::fs::write(&path, vec![b'x'; len]).expect("file");
    path
}

/// What the fake recorded of the submission: message and object ids.
type Sent = Arc<Mutex<Option<(String, String)>>>;

fn listed_messages(sent: &Sent) -> Value {
    let mut frame = frames()["message.list"].clone();
    if let Some((message, _)) = sent.lock().expect("sent").clone() {
        frame["messages"].as_array_mut().expect("messages").push(json!({
            "conversation_id": "dm-ana", "message_id": message, "sender_id": "device-me",
            "encrypted_body": [], "deleted": false, "created_at": 1_790_000_300, "receipts": [],
        }));
    }
    frame
}

fn listed_objects(sent: &Sent) -> Value {
    let mut frame = frames()["object.directory"].clone();
    if let Some((message, object)) = sent.lock().expect("sent").clone() {
        frame["objects"].as_array_mut().expect("objects").push(json!({
            "id": object, "size": 8192, "transfer_state": "available", "name": "report.txt",
            "direction": "outgoing", "conversation_id": "dm-ana", "message_id": message,
            "transfer": null,
        }));
    }
    frame
}

/// The message this device is sending with an attachment, if any.
fn sending(store: &Store) -> Option<(Moment, MessagePayload)> {
    store.moments().find_map(|m| match &m.kind {
        MomentKind::Message(p) if p.attachment.is_some() && m.origin.as_str() == "principal-me" => {
            Some((m.clone(), p.clone()))
        }
        _ => None,
    })
}

#[test]
fn an_attachment_shows_its_transfer_then_its_message() {
    let path = socket_path("attachok");
    let sent: Sent = Arc::new(Mutex::new(None));
    let record = sent.clone();
    let fake = FakeBus::start(&path, move |method, _, body| match method {
        "message.submit" => {
            // Slow enough to see the transfer while it runs; like the
            // daemon, the message and object are listed once it succeeded.
            std::thread::sleep(Duration::from_millis(600));
            let message = body["message_id"].as_str().unwrap_or_default().to_owned();
            let object = body["attachments"][0]["object_id"].as_str().unwrap_or_default();
            *record.lock().expect("sent") = Some((message.clone(), object.to_owned()));
            Reply::Ok(json!({ "message_id": message }))
        }
        "message.list" => Reply::Ok(listed_messages(&record)),
        "object.directory" => Reply::Ok(listed_objects(&record)),
        _ => fixture(method),
    });
    let report = file("report", 8192);
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("the relay and the conversation", |s, _| {
        s.context(&ContextId::new("dm-ana")).is_some()
    });
    account.source.focus(Some(&ContextId::new("dm-ana")));
    account.source.submit(attach(&report));
    account.until("the message on its way", |s, _| sending(s).is_some());
    let (moment, payload) = sending(&account.store).expect("message");
    assert_eq!(moment.lifecycle, Lifecycle::InProgress);
    assert_eq!(payload.receipt, Receipt::Sending);
    let object_id = payload.attachment.clone().expect("attachment");
    let object = account.store.object(&object_id).cloned().expect("object");
    assert_eq!(object.direction, Direction::Outbound);
    assert_eq!(object.name.as_deref(), Some("report.txt"));
    assert_eq!(object.size, Some(8192), "the size of the file sent");
    assert_eq!(object.transfer, TransferState::Transferring { transferred: 0 }, "no bytes claimed");
    // "Sending" shows before the request reaches the fake; wait for it to arrive.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let submitted = loop {
        if let Some(seen) = fake.seen().into_iter().find(|s| s.method == "message.submit") {
            break seen.body;
        }
        assert!(std::time::Instant::now() < deadline, "submitted");
        std::thread::sleep(Duration::from_millis(10));
    };
    let message = submitted["message_id"].as_str().expect("message id").to_owned();
    assert_eq!(moment.id, MomentId::new(format!("msg:dm-ana:{message}")), "the message's own id");
    assert_eq!(
        object_id,
        ObjectId::new(submitted["attachments"][0]["object_id"].as_str().expect("id"))
    );

    account.until("the listed message, sent", |s, _| {
        s.moment(&moment.id).is_some_and(|m| {
            m.lifecycle == Lifecycle::Done
                && matches!(&m.kind, MomentKind::Message(p)
                    if p.receipt == Receipt::Sent && p.attachment.as_ref() == Some(&object_id))
        })
    });
    account.until("the object listed as sent", |s, _| {
        s.object(&object_id).is_some_and(|o| o.transfer == TransferState::Complete)
    });
    assert!(account.store.failures().is_empty(), "{:?}", account.store.failures());
    drop(account);
    fake.shutdown();
    let _ = std::fs::remove_dir_all(report.parent().expect("dir"));
}

#[test]
fn a_failed_attachment_shows_the_reason_on_its_transfer() {
    let path = socket_path("attachko");
    let fake = FakeBus::start(&path, |method, _, _| match method {
        "message.submit" => Reply::Error(
            "ValidationFailed",
            "local bus error: relay object upload failed".to_owned(),
        ),
        _ => fixture(method),
    });
    let report = file("broken", 64);
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("the conversation", |s, _| s.context(&ContextId::new("dm-ana")).is_some());
    account.source.submit(attach(&report));
    account.until("the failure", |s, _| !s.failures().is_empty());
    account.until("the failed message", |s, _| {
        sending(s).is_some_and(|(m, _)| m.lifecycle == Lifecycle::Failed)
    });
    let (_, payload) = sending(&account.store).expect("message");
    let object_id = payload.attachment.expect("attachment");
    let object = account.store.object(&object_id).cloned().expect("object");
    assert!(
        matches!(&object.transfer, TransferState::Failed { reason } if reason.contains("relay object upload failed")),
        "{:?}",
        object.transfer
    );
    assert_eq!(
        account.store.failures().last().map(|f| f.method.as_str()),
        Some(methods::ATTACH_OBJECT)
    );
    drop(account);
    fake.shutdown();
    let _ = std::fs::remove_dir_all(report.parent().expect("dir"));
}

#[test]
fn a_file_over_the_frame_budget_is_refused_before_sending() {
    let path = socket_path("attachbig");
    let fake = FakeBus::start(&path, |method, _, _| fixture(method));
    let big = file("big", ramflux_desktop_sdk::ATTACHMENT_LIMIT + 1);
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("the conversation", |s, _| s.context(&ContextId::new("dm-ana")).is_some());
    account.source.submit(attach(&big));
    account.until("the refusal", |s, _| !s.failures().is_empty());
    let failure = account.store.failures().last().cloned().expect("failure");
    assert!(
        failure.message.starts_with("backend.failure.attachment_too_large"),
        "{}",
        failure.message
    );
    assert!(sending(&account.store).is_none(), "nothing is shown as sending");
    assert!(!fake.seen().iter().any(|s| s.method == "message.submit"), "nothing was sent");
    drop(account);
    fake.shutdown();
    let _ = std::fs::remove_dir_all(big.parent().expect("dir"));
}
