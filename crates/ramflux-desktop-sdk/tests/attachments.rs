// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Receiving messages never depends on attachments: the pull asks the
//! daemon not to download them, a failing pull never stops the read of the
//! stored messages, and an object appears when the object directory lists
//! it, without touching the conversation.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::*;
use ramflux_desktop_core::{
    AccountId, ContextId, DomainEvent, MomentId, ObjectId, Source as _, SourceStatus, TransferState,
};
use serde_json::{Value, json};

/// The fixture's messages of `dm-ana`, plus `m3` once `later` is set.
fn messages(later: bool) -> Value {
    let mut frame = frames()["message.list"].clone();
    if later {
        let mut m3 = frame["messages"][0].clone();
        m3["message_id"] = json!("m3");
        m3["encrypted_body"] = json!([108, 97, 116, 101, 114]);
        m3["created_at"] = json!(1_790_000_120);
        frame["messages"].as_array_mut().expect("messages").push(m3);
    }
    frame
}

#[test]
fn a_failing_pull_never_holds_back_the_conversation() {
    let path = socket_path("pullfail");
    let pulls = Arc::new(AtomicUsize::new(0));
    let counter = pulls.clone();
    let fake = FakeBus::start(&path, move |method, _, _| match method {
        // As a receive that tried to download an attachment and failed.
        "message.receive" => {
            counter.fetch_add(1, Ordering::SeqCst);
            Reply::Error(
                "ValidationFailed",
                "local bus error: relay object download failed".to_owned(),
            )
        }
        "message.list" => Reply::Ok(messages(counter.load(Ordering::SeqCst) >= 2)),
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.source.focus(Some(&ContextId::new("dm-ana")));
    account.until("a later message after failed pulls", |s, _| {
        s.moment(&MomentId::new("msg:dm-ana:m3")).is_some()
    });
    assert_eq!(account.store.body(&MomentId::new("msg:dm-ana:m3")), Some("later"));
    assert!(
        account.store.failures().is_empty(),
        "a poll is not an effect: {:?}",
        account.store.failures()
    );
    assert!(
        !account.envelopes.iter().any(|e| matches!(
            e.event,
            DomainEvent::SourceStatusChanged(SourceStatus::Disconnected)
        )),
        "a refused pull is not a lost daemon"
    );
    let seen = fake.seen();
    assert!(
        seen.iter()
            .filter(|s| s.method == "message.receive")
            .all(|s| s.body["auto_fetch_attachments"] == false),
        "the pull never downloads attachments"
    );
    // Every failed pull is followed by the read of the stored messages.
    let order: Vec<&str> = seen
        .iter()
        .filter(|s| matches!(s.method.as_str(), "message.receive" | "message.list"))
        .map(|s| s.method.as_str())
        .collect();
    let pulls_followed =
        order.windows(2).filter(|w| w == &["message.receive", "message.list"]).count();
    assert!(pulls_followed >= 2, "{order:?}");
    fake.shutdown();
}

#[test]
fn an_object_appears_when_the_directory_lists_it_without_touching_the_conversation() {
    let path = socket_path("objlater");
    let polls = Arc::new(AtomicUsize::new(0));
    let counter = polls.clone();
    let fake = FakeBus::start(&path, move |method, _, _| match method {
        "object.directory" => {
            let mut frame = frames()[method].clone();
            if counter.fetch_add(1, Ordering::SeqCst) < 2 {
                frame["objects"]
                    .as_array_mut()
                    .expect("rows")
                    .retain(|r| r["id"] != "attachment:m1:0");
            }
            Reply::Ok(frame)
        }
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.source.focus(Some(&ContextId::new("dm-ana")));
    account.until("messages", |s, _| s.moment(&MomentId::new("msg:dm-ana:m1")).is_some());
    let object = ObjectId::new("attachment:m1:0");
    account.until("the object, listed later", |s, _| {
        s.object(&object).is_some_and(|o| o.transfer == TransferState::Complete)
    });
    assert!(account.store.rejections().is_empty(), "{:?}", account.store.rejections());
    assert!(account.store.failures().is_empty());
    fake.shutdown();
}
