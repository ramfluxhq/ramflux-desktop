// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A conversation someone else starts appears without any desktop change
//! once the daemon lists it: a pushed `conversation.changed` (or a
//! `message.received` naming an unknown conversation) re-reads the
//! conversation directory at once, and the new row becomes a context. The
//! poll is slowed to a minute here, so only the push can explain it.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use common::*;
use ramflux_desktop_core::{AccountId, ContextId, ContextKind};
use serde_json::{Value, json};

fn conversations(with_new: bool) -> Value {
    let mut frame = frames()["conversation.directory"].clone();
    if with_new {
        let mut row = frame["conversations"][0].clone();
        row["id"] = json!("dm-from-zoe");
        row["participants"] = json!(["principal-me", "principal-zoe"]);
        frame["conversations"].as_array_mut().expect("rows").push(row);
    }
    frame
}

fn slow_poll(path: &std::path::Path) -> ramflux_desktop_sdk::SdkConfig {
    ramflux_desktop_sdk::SdkConfig {
        poll_fast: Duration::from_secs(60),
        poll_slow: Duration::from_secs(60),
        ..config(path)
    }
}

fn appears_on(topic: &str, body: Value) {
    let path = socket_path(&format!("stream-{}", topic.replace('.', "-")));
    let listed = Arc::new(AtomicBool::new(false));
    let flag = listed.clone();
    let fake = FakeBus::start(&path, move |method, _, _| match method {
        "conversation.directory" => Reply::Ok(conversations(flag.load(Ordering::SeqCst))),
        _ => fixture(method),
    });
    let hub = ramflux_desktop_sdk::SdkHub::start(slow_poll(&path)).expect("bridge");
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("subscribed", |s, _| {
        s.context(&ContextId::new("dm-ana")).is_some() && fake.subscribed("acct-main")
    });
    let opened = fake.seen().into_iter().find(|s| s.method == "subscription.open").expect("open");
    let topics: Vec<&str> = opened.body["topics"]
        .as_array()
        .expect("topics")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(topics.contains(&topic), "{topic} is subscribed: {topics:?}");
    let new = ContextId::new("dm-from-zoe");
    account.settle(200);
    assert!(account.store.context(&new).is_none());
    listed.store(true, Ordering::SeqCst);
    fake.push("acct-main", topic, body);
    account.until("the new conversation", |s, _| s.context(&new).is_some());
    assert_eq!(account.store.context(&new).map(|c| c.kind), Some(ContextKind::Direct));
    drop(account);
    fake.shutdown();
}

#[test]
fn a_new_conversation_appears_on_conversation_changed() {
    appears_on("conversation.changed", json!({ "conversation_id": "dm-from-zoe" }));
}

#[test]
fn a_message_in_an_unknown_conversation_reads_the_directory() {
    appears_on(
        "message.received",
        json!({ "conversation_id": "dm-from-zoe", "message_id": "m-zoe-1" }),
    );
}
