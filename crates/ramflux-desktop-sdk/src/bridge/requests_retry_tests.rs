// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Bounded read recovery, cancellation, coalescing and local readiness.

use super::*;
use ramflux_desktop_core::{AccountId, EventEnvelope, LockState, Store, SyncMode};
use ramflux_desktop_test::fake_bus::{FakeBus, Reply, socket_path};
use serde_json::json;

fn task() -> (Task, Inbox, std_mpsc::Receiver<Delivery>) {
    let mut config = SdkConfig::new("retry-test.sock");
    config.call_api = false;
    config.backup_api = false;
    let (outbox, delivered) = std_mpsc::channel();
    let (phrases, _) = std_mpsc::channel();
    let (mut task, inbox) = build(
        config,
        Scope::Account(AccountId::new("acct-main")),
        outbox,
        Ceremonies {
            phrases: phrases.into(),
            audit: Arc::new(super::super::PhraseAudit::default()),
        },
    );
    task.link = Link::Live;
    task.sync.mode = SyncMode::Stream;
    (task, inbox, delivered)
}

fn apply(delivered: &std_mpsc::Receiver<Delivery>, store: &mut Store, seq: &mut u64) {
    for delivery in delivered.try_iter() {
        *seq += 1;
        store.apply(&EventEnvelope::new(*seq, delivery.at, delivery.event)).expect("apply");
    }
}

async fn answer(task: &mut Task, inbox: &mut Inbox) {
    let answer = tokio::time::timeout(Duration::from_secs(3), inbox.internal.recv())
        .await
        .expect("answer deadline")
        .expect("internal answer");
    task.internal(answer);
}

// Observe the server recording each request before counting it. A client timeout
// can otherwise precede the fake daemon's thread being scheduled.
async fn observed_request(observed: &mut mpsc::UnboundedReceiver<String>, method: &str) {
    let recorded = tokio::time::timeout(Duration::from_secs(3), observed.recv())
        .await
        .expect("request observation deadline")
        .expect("recorded request");
    assert_eq!(recorded, method);
}

#[tokio::test]
async fn read_timeout_keeps_rows_and_retries_once() {
    let path = socket_path("bounded-read-timeout");
    let (recorded, mut observed) = mpsc::unbounded_channel();
    let fake = FakeBus::start(&path, move |method, _, _| {
        recorded.send(method.to_owned()).expect("request observer");
        Reply::Slow(Duration::from_secs(2), json!({}))
    });
    let (mut task, mut inbox, delivered) = task();
    task.client = BusClient::new(
        path,
        Duration::from_secs(2),
        crate::config::Deadlines::uniform(Duration::from_millis(10)),
    );
    for fetch in [Fetch::ContactRequests, Fetch::ContactList, Fetch::NodeList] {
        let mut store = Store::new();
        let mut seq = 0;
        task.on_read(&Fetch::ContactRequests, Ok(json!({"requests": [{
            "request_id":"kept", "principal_id":"peer", "direction":"inbound", "state":"pending", "note":"keep me"
        }]})));
        apply(&delivered, &mut store, &mut seq);
        assert_eq!(store.contact_requests().count(), 1);
        let before = Instant::now();
        let baseline = fake.seen().len();
        let method = fetch.request(Some("acct-main")).method;
        task.spawn_fetches(vec![fetch.clone(), fetch.clone()]);
        observed_request(&mut observed, &method).await;
        assert!(observed.try_recv().is_err(), "no extra request frame");
        answer(&mut task, &mut inbox).await;
        apply(&delivered, &mut store, &mut seq);
        assert!(store.failures().is_empty(), "first failure stays silent");
        assert!(store.contact_requests_read_error().is_none());
        assert_eq!(store.contact_requests().count(), 1);
        let due = task.read_recovery.next_due().expect("one retry armed");
        assert!(due >= before + Duration::from_secs(1));
        assert!(due <= Instant::now() + Duration::from_secs(1));
        let sent = fake.seen().len();
        assert_eq!(sent, baseline + 1, "duplicate fetches send one initial request");
        task.tick();
        assert_eq!(fake.seen().len(), sent, "no immediate retry");
        tokio::time::sleep_until(due).await;
        task.tick();
        observed_request(&mut observed, &method).await;
        assert!(observed.try_recv().is_err(), "no extra request frame");
        answer(&mut task, &mut inbox).await;
        apply(&delivered, &mut store, &mut seq);
        assert!(task.read_recovery.next_due().is_none(), "second timeout exhausts retry");
        assert_eq!(store.contact_requests().count(), 1, "last good rows survive both timeouts");
        assert_eq!(
            store.contact_requests().next().expect("kept row").note.as_deref(),
            Some("keep me")
        );
        if fetch == Fetch::ContactRequests {
            assert!(store.contact_requests_read_error().is_some(), "second timeout reported");
        } else {
            assert!(!store.failures().is_empty(), "second timeout reported");
        }
        for _ in 0..3 {
            task.tick();
        }
        assert!(task.read_recovery.running.is_empty(), "no retry spawned after exhaustion");
        assert!(task.read_recovery.rerun.is_empty(), "no follow-up queued");
        assert_eq!(fake.seen().len(), sent + 1, "only one retry request");
        assert!(inbox.internal.try_recv().is_err());
        task.spawn_reads(vec![fetch.clone()], false);
        assert!(task.read_recovery.running.is_empty(), "timer cannot rearm exhausted reads");
        // Only a new event/action can rearm an exhausted cycle.
        task.spawn_fetches(vec![fetch]);
        observed_request(&mut observed, &method).await;
        assert!(observed.try_recv().is_err(), "no extra request frame");
        answer(&mut task, &mut inbox).await;
        assert!(task.read_recovery.next_due().is_some(), "new action rearms recovery");
        task.read_recovery.cancel();
    }
    fake.shutdown();
}

#[tokio::test]
async fn read_retry_cancels_on_lock_or_detach_and_coalesces() {
    let path = socket_path("bounded-read-cancel");
    let fake = FakeBus::start(&path, |_, _, _| {
        Reply::Slow(Duration::from_millis(80), json!({"requests": []}))
    });
    for cancel in 0..3 {
        let (mut task, mut inbox, _delivered) = task();
        task.client = BusClient::new(
            path.clone(),
            Duration::from_secs(2),
            crate::config::Deadlines::standard(),
        );
        let baseline = fake.seen().len();
        task.spawn_fetches(vec![Fetch::ContactRequests; 3]);
        assert_eq!(task.read_recovery.running.len(), 1);
        answer(&mut task, &mut inbox).await;
        assert_eq!(fake.seen().len(), baseline + 1, "concurrent refreshes coalesce");
        task.on_read(&Fetch::ContactRequests, Err(BusError::Timeout));
        let due = task.read_recovery.next_due().expect("retry");
        tokio::time::sleep_until(due).await;
        task.tick();
        answer(&mut task, &mut inbox).await;
        assert!(task.read_recovery.next_due().is_none(), "success ends recovery");
        task.tick();
        assert_eq!(fake.seen().len(), baseline + 2);
        // A new event can rearm, and concurrent event refreshes coalesce.
        task.on_pushed(Pushed::Event { topic: "contact.requested".into(), body: json!({}) });
        task.on_pushed(Pushed::Event { topic: "contact.requested".into(), body: json!({}) });
        assert_eq!(task.read_recovery.running.len(), 1);
        answer(&mut task, &mut inbox).await;
        assert_eq!(fake.seen().len(), baseline + 3);
        answer(&mut task, &mut inbox).await;
        assert_eq!(fake.seen().len(), baseline + 4, "one follow-up for the later event");
        task.on_read(&Fetch::ContactRequests, Err(BusError::Timeout));
        let cancel_due = task.read_recovery.next_due().expect("new episode");
        // Also cancel an active fetch, including a late completion already queued.
        task.spawn_fetches(vec![Fetch::ContactList]);
        task.spawn_fetches(vec![Fetch::ContactList]);
        assert!(task.read_recovery.rerun.contains(&Fetch::ContactList));
        let handle = task.read_recovery.running[&Fetch::ContactList].clone();
        match cancel {
            0 => task.locked(),
            1 => task.lost(),
            _ => {
                drop(task);
                tokio::task::yield_now().await;
                assert!(handle.is_finished());
                tokio::time::sleep_until(cancel_due).await;
                assert_eq!(
                    fake.seen().iter().filter(|s| s.method == "contact.requests.list").count(),
                    (cancel as usize + 1) * 4
                );
                continue;
            }
        }
        assert!(task.read_recovery.next_due().is_none());
        assert!(task.read_recovery.rerun.is_empty(), "cancel also clears pending follow-ups");
        tokio::time::sleep_until(cancel_due).await;
        // Keep the disconnected task before its reconnect deadline.
        task.reconnect_at = Instant::now() + Duration::from_secs(60);
        task.tick();
        assert!(task.read_recovery.running.is_empty());
        task.internal(Internal::Fetched((Fetch::ContactList, None, Err(BusError::Timeout))));
        assert!(task.read_recovery.next_due().is_none(), "late completion cannot rearm");
    }
    fake.shutdown();
}

#[tokio::test]
async fn unlocked_status_is_delivered_before_slow_snapshot_lists() {
    let path = socket_path("local-readiness");
    let fake = FakeBus::start(&path, |method, _, _| {
        if method == "account.status" {
            Reply::Ok(json!({"state":"unlocked", "networking":"networking_pending"}))
        } else {
            Reply::Slow(Duration::from_secs(2), json!({}))
        }
    });
    let (mut task, mut inbox, delivered) = task();
    task.client =
        BusClient::new(path, Duration::from_secs(2), crate::config::Deadlines::standard());
    task.snapshot();
    // The epoch marker comes first in the channel; the status answer follows.
    while task.link != Link::Live {
        answer(&mut task, &mut inbox).await;
    }
    assert!(task.snapshot_task.is_some(), "other snapshot reads still pending");
    let events: Vec<_> = delivered.try_iter().map(|delivery| delivery.event).collect();
    assert!(events.iter().any(|event| matches!(event,
        DomainEvent::AccountUpdated(account) if account.lock_state == LockState::Unlocked)));
    assert!(
        events.iter().any(|event| matches!(
            event,
            DomainEvent::SourceStatusChanged(SourceStatus::Connected)
        ))
    );
    task.locked();
    fake.shutdown();
}
