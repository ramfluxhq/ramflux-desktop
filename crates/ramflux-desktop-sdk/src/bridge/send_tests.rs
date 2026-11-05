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
        .filter_map(|d| {
            let (kind, lifecycle) = match d.event {
                DomainEvent::MomentAdded(m) => (Some(m.kind), Some(m.lifecycle)),
                DomainEvent::MomentUpdated { kind, lifecycle, .. } => (kind, lifecycle),
                _ => return None,
            };
            let Some(ramflux_desktop_core::MomentKind::Message(p)) = kind else { return None };
            let expected = match p.receipt {
                Receipt::Sent | Receipt::Delivered | Receipt::Read => {
                    ramflux_desktop_core::Lifecycle::Done
                }
                Receipt::Failed { .. } => ramflux_desktop_core::Lifecycle::Failed,
                _ => ramflux_desktop_core::Lifecycle::InProgress,
            };
            assert_eq!(lifecycle, Some(expected), "receipt and lifecycle must agree");
            Some(p.receipt)
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
    tokio::time::pause();
    for _ in 0..3 {
        let due = task.next_send_due().expect("each refusal rearms");
        tokio::time::advance(due.saturating_duration_since(Instant::now())).await;
        task.retry_sends(false);
        tokio::time::resume();
        let _ = inbox.internal.recv().await.expect("retry reached bus");
        tokio::time::pause();
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

#[tokio::test]
async fn waiting_send_fails_after_ten_minutes_and_manual_retry_uses_new_key() {
    use ramflux_desktop_core::{
        EventEnvelope, Intent, IntentKind, MomentKind, Store, Timestamp, handle_intent,
    };
    let (mut task, mut inbox, rx, fake) = fixture("waiting-expiry");
    let frames: std::collections::BTreeMap<String, Value> =
        serde_json::from_str(include_str!("../../tests/fixtures/rfd_frames.json")).expect("frames");
    for (fetch, key) in [
        (Fetch::AccountStatus(AccountId::new("a")), "account.status"),
        (Fetch::DeviceList, "device.list"),
        (Fetch::first(crate::plan::fetch::Paged::Identities), "identity.directory"),
        (Fetch::first(crate::plan::fetch::Paged::Conversations), "conversation.directory"),
    ] {
        let out = task.translator.fetched(&fetch, &frames[key], Timestamp(0));
        task.emit(out.events);
    }
    task.start_send(
        json!({"context_id":"dm-ana","text":"hello"}),
        BusRequest::new(
            Some("a"),
            "message",
            "message.submit",
            json!({"message_id":"m","conversation_id":"dm-ana"}),
        )
        .with_operation_id("original".to_owned()),
        Vec::new(),
    );
    let _ = inbox.internal.recv().await.expect("first");
    tokio::time::pause();
    task.send_done("m", Err(refusal(Some(600_000))));
    tokio::time::advance(Duration::from_secs(599)).await;
    task.retry_sends(false);
    assert!(task.sends.contains_key("m"), "not before ten minutes");
    tokio::time::advance(Duration::from_secs(1)).await;
    task.retry_sends(false);
    assert!(task.sends.is_empty());
    let mut store = Store::new();
    for (index, d) in rx.try_iter().enumerate() {
        store.apply(&EventEnvelope::new(index as u64 + 1, d.at, d.event)).expect("project");
    }
    let moment = store.moments().find(|m| matches!(&m.kind, MomentKind::Message(p) if p.receipt == (Receipt::Failed { retryable: true }))).expect("failed").id.clone();
    let effects = handle_intent(
        &store,
        Intent::human(IntentKind::RetryMessage { moment_id: moment }),
        Timestamp(0),
    )
    .expect("manual retry");
    tokio::time::resume();
    for effect in effects {
        task.effect(effect);
    }
    let Internal::SendDone { .. } =
        tokio::time::timeout(Duration::from_secs(3), inbox.internal.recv())
            .await
            .expect("manual send deadline")
            .expect("manual request")
    else {
        panic!("send")
    };
    let seen = fake.seen();
    assert_eq!(seen.len(), 2);
    assert_ne!(seen[0].operation_id, seen[1].operation_id);
    assert_ne!(
        seen[0].body["message_id"], seen[1].body["message_id"],
        "manual retry is a new action"
    );
}

#[tokio::test]
async fn timeout_send_is_never_retried_with_fresh_key() {
    for (name, error) in [
        ("send-timeout", BusError::Timeout),
        ("send-transport", BusError::Transport("answer lost".into())),
    ] {
        let (mut task, mut inbox, rx, fake) = fixture(name);
        start(&mut task);
        let _ = inbox.internal.recv().await.expect("first");
        task.send_done("m", Err(error));
        assert_eq!(receipt(&rx), Receipt::Failed { retryable: false });
        task.networking_event(&json!({"account_id":"a","state":"live","session":{"state":"live"}}));
        task.retry_sends(true);
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert_eq!(fake.seen().len(), 1, "no fresh operation after an ambiguous outcome");
        assert!(task.next_send_due().is_none());
        assert!(task.sends.is_empty());
        fake.shutdown();
    }
}

#[tokio::test]
async fn waiting_send_survives_lost_and_attaching_links_until_live_or_expiry() {
    for link in [Link::Lost, Link::Attaching] {
        for expire in [false, true] {
            let (mut task, mut inbox, rx, fake) = fixture(&format!("send-{link:?}-{expire}"));
            start(&mut task);
            let _ = inbox.internal.recv().await.expect("first");
            task.send_done("m", Err(refusal(Some(1_000))));
            task.link = link;
            let expiry = task.sends["m"].started + Duration::from_secs(600);
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(1)).await;
            task.retry_sends(false);
            assert!(matches!(receipt(&rx), Receipt::WaitingForNode { .. }));
            assert_eq!(fake.seen().len(), 1, "offline link dispatches nothing");
            assert_eq!(task.next_send_due(), Some(expiry), "no busy wake on overdue retry");
            assert_eq!(task.sends["m"].request.operation_id.as_deref(), Some("original"));
            if expire {
                tokio::time::advance(expiry.saturating_duration_since(Instant::now())).await;
                task.retry_sends(false);
                assert_eq!(receipt(&rx), Receipt::Failed { retryable: true });
                assert!(task.sends.is_empty());
                assert_eq!(fake.seen().len(), 1);
                tokio::time::resume();
            } else {
                task.link = Link::Live;
                task.retry_sends(false);
                tokio::time::resume();
                let Internal::SendDone { id, result } =
                    tokio::time::timeout(Duration::from_secs(3), inbox.internal.recv())
                        .await
                        .expect("retry deadline")
                        .expect("retry")
                else {
                    panic!("send")
                };
                task.send_done(&id, result);
                assert_eq!(receipt(&rx), Receipt::Sent);
                assert_eq!(fake.seen().len(), 2);
            }
            fake.shutdown();
        }
    }
}

#[tokio::test]
async fn unresolved_group_members_are_terminal_even_with_networking_details() {
    let (mut task, mut inbox, rx, fake) = fixture("unresolved-group-send");
    start(&mut task);
    let _ = inbox.internal.recv().await.expect("first request");
    let error = BusError::from_error_body(Some(ramflux_sdk::LocalBusErrorBody {
        code: "ValidationFailed".into(),
        message: r#"local bus error: group routes unresolved: {"group_id":"g-team","unreachable_members":["device-bo","device-zz"]}"#.into(),
        retry_after_ms: Some(1),
        reason_code: None,
        cause: None,
        details: Some(Box::new(json!({"networking":{"state":"networking_pending"}}))),
    }));
    task.send_done("m", Err(error));
    assert_eq!(receipt(&rx), Receipt::Failed { retryable: false });
    assert!(task.sends.is_empty(), "terminal refusals retain no retry entry");
    assert!(task.next_send_due().is_none(), "no retry or expiry timer");
    task.networking_event(&json!({"account_id":"a","state":"live","session":{"state":"live"}}));
    task.retry_sends(true);
    assert!(task.sends.is_empty());
    assert!(task.next_send_due().is_none());
    assert_eq!(fake.seen().len(), 1, "Live does not retry the refused request");
    fake.shutdown();
}
