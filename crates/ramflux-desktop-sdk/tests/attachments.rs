// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Receiving messages never depends on attachments: a failing pull never
//! stops the read of the stored messages (after the first page, only the
//! rows changed after its cursor are read).

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::*;
use ramflux_desktop_core::{
    AccountId, ContextId, DomainEvent, MomentId, Source as _, SourceStatus,
};
use serde_json::{Value, json};

/// The fixture's messages of `dm-ana`, plus `m3` once `later` is set.
fn messages(later: bool) -> Value {
    let mut frame = frames()["message.read"].clone();
    if later {
        // Newest first: m2 (mine), then m1 (Ana's).
        let mut m3 = frame["messages"][1].clone();
        m3["id"] = json!("m3");
        m3["text"] = json!("later");
        m3["hlc_ms"] = json!(1_790_000_120_000_u64);
        frame["messages"].as_array_mut().expect("messages").insert(0, m3);
    }
    frame
}

#[test]
fn a_failing_pull_never_holds_back_the_conversation() {
    let path = socket_path("pullfail");
    let pulls = Arc::new(AtomicUsize::new(0));
    let counter = pulls.clone();
    let fake = FakeBus::start(&path, move |method, _, body| match method {
        // As a receive that tried to download an attachment and failed.
        "message.receive" => {
            counter.fetch_add(1, Ordering::SeqCst);
            Reply::Error(
                "ValidationFailed",
                "local bus error: relay object download failed".to_owned(),
            )
        }
        "message.read" if body.get("after_sequence").is_some() => {
            let page = messages(counter.fetch_add(1, Ordering::SeqCst) >= 2);
            let changes: Vec<Value> =
                page["messages"].as_array().into_iter().flatten().take(1).cloned().collect();
            Reply::Ok(json!({ "schema": page["schema"], "conversation_id": "dm-ana",
                              "changes": changes, "next_after": 3, "more": false,
                              "snapshot_sequence": 3 }))
        }
        "message.read" => Reply::Ok(messages(counter.fetch_add(1, Ordering::SeqCst) >= 2)),
        _ => fixture(method),
    });
    fake.enable_stream();
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.source.focus(Some(&ContextId::new("dm-ana")));
    account.until("stream", |_, _| fake.streaming("acct-main"));
    account.settle(50);
    fake.publish("acct-main", "message.received", json!({"conversation_id":"dm-ana"}));
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
        !seen.iter().any(|s| s.method == "message.receive"),
        "desktop never pulls from the node"
    );
    assert!(seen.iter().filter(|s| s.method == "message.read").count() >= 2);
    fake.shutdown();
}
