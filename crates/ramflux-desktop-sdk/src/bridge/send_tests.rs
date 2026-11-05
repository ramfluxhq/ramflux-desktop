// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Deterministic send deadlines and real fake-bus request identities.
use super::super::scope::{Ceremonies, Inbox, Link, build};
use super::*;
use crate::config::SdkConfig;
use crate::translate::Scope;
use ramflux_desktop_core::{AccountId, DomainEvent};
use ramflux_desktop_test::fake_bus::{FakeBus, Reply, socket_path};
use serde_json::json;
use std::sync::{Arc, mpsc};

fn fixture(name: &str) -> (Task, Inbox, mpsc::Receiver<super::super::Delivery>, FakeBus) {
    let path = socket_path(name);
    let fake = FakeBus::start(&path, |_, _, _| Reply::Ok(json!({"message_id":"m"})));
    let mut config = SdkConfig::new(path);
    config.settings_path = None;
    config.backup_api = false;
    let (tx, rx) = mpsc::channel();
    let (phrase, _) = mpsc::channel();
    let (mut task, inbox) = build(
        config,
        Scope::Account(AccountId::new("a")),
        tx,
        Ceremonies { phrases: phrase.into(), audit: Arc::default() },
    );
    task.link = Link::Live;
    (task, inbox, rx, fake)
}

fn start(task: &mut Task) {
    task.start_send(
        json!({"context_id":"c","text":"hello"}),
        BusRequest::new(
            Some("a"),
            "message",
            "message.submit",
            json!({"message_id":"m","conversation_id":"c"}),
        )
        .with_operation_id("original".to_owned()),
        Vec::new(),
    );
}

fn refusal(hint: Option<u64>) -> BusError {
    BusError::from_error_body(Some(ramflux_sdk::LocalBusErrorBody {
        code: "NetworkingPending".into(),
        message: "wording is irrelevant".into(),
        retry_after_ms: hint,
        reason_code: None,
        cause: None,
        details: Some(Box::new(json!({"networking":{"state":"networking_pending"}}))),
    }))
}

fn receipt(rx: &mpsc::Receiver<super::super::Delivery>) -> Receipt {
    rx.try_iter()
        .filter_map(|d| match d.event {
            DomainEvent::MomentAdded(m) => Some(m.kind),
            DomainEvent::MomentUpdated { kind, .. } => kind,
            _ => None,
        })
        .filter_map(|k| match k {
            ramflux_desktop_core::MomentKind::Message(p) => Some(p.receipt),
            _ => None,
        })
        .last()
        .expect("receipt")
}

#[tokio::test]
async fn send_while_offline_shows_waiting_with_retry_countdown_then_sends_on_live() {
    let (mut task, mut inbox, rx, fake) = fixture("offline-send");
    start(&mut task);
    // Complete the first dispatched request before injecting the typed refusal.
    let _ = inbox.internal.recv().await.expect("first request");
    task.send_done("m", Err(refusal(Some(5_000))));
    assert!(matches!(receipt(&rx), Receipt::WaitingForNode { .. }));
    let due = task.next_send_due().expect("one-shot");
    assert!(due > Instant::now() + Duration::from_secs(4));
    task.retry_sends(false);
    assert_eq!(fake.seen().len(), 1, "not before deadline");
    task.networking_event(&json!({"account_id":"a","state":"live","session":{"state":"live"}}));
    let Internal::SendDone { id, result } = inbox.internal.recv().await.expect("retry") else {
        panic!("send")
    };
    task.send_done(&id, result);
    assert_eq!(receipt(&rx), Receipt::Sent);
    let seen = fake.seen();
    assert_eq!(seen.len(), 2);
    assert_ne!(seen[0].operation_id, seen[1].operation_id);
    assert_eq!(seen[0].body, seen[1].body, "same logical message");
}

#[tokio::test]
async fn repeated_pending_reschedules_without_live_event() {
    let (mut task, mut inbox, rx, fake) = fixture("repeat-pending");
    start(&mut task);
    let _ = inbox.internal.recv().await.expect("first");
    // Account may already be Live when the method refuses pre-acceptance.
    task.networking_event(&json!({"account_id":"a","state":"live","session":{"state":"live"}}));
    task.send_done("m", Err(refusal(Some(1))));
    for _ in 0..3 {
        let due = task.next_send_due().expect("each refusal rearms");
        tokio::time::sleep_until(due).await;
        task.retry_sends(false);
        let _ = inbox.internal.recv().await.expect("retry reached bus");
        task.send_done("m", Err(refusal(Some(1))));
        assert!(matches!(receipt(&rx), Receipt::WaitingForNode { .. }));
    }
    assert_eq!(fake.seen().len(), 4);
    let keys: std::collections::BTreeSet<_> =
        fake.seen().iter().map(|r| r.operation_id.clone()).collect();
    assert_eq!(keys.len(), 4);
}

#[tokio::test]
async fn pending_without_hint_follows_ladder() {
    let (mut task, mut inbox, _rx, _fake) = fixture("ladder-pending");
    start(&mut task);
    let _ = inbox.internal.recv().await.expect("first");
    for delay in [2_000, 5_000, 15_000, 30_000, 60_000, 60_000] {
        let before = Instant::now();
        task.send_done("m", Err(refusal(None)));
        let after = Instant::now();
        let due = task.next_send_due().expect("deadline");
        assert!(due >= before + Duration::from_millis(delay));
        assert!(due <= after + Duration::from_millis(delay));
    }
    assert_eq!(
        pending(&BusError::daemon("NetworkingPending", "networking words")),
        None,
        "details required"
    );
    let mut wrong = refusal(Some(1));
    if let BusError::Daemon { code, .. } = &mut wrong {
        *code = "Other".into();
    }
    assert_eq!(pending(&wrong), None, "code required");
}
