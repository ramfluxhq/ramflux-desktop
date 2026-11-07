// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Review regressions: failure episodes, permanent answers and lost updates.
use super::*;
use ramflux_desktop_core::{AccountId, EventEnvelope, Listing, Store, SyncMode};
use ramflux_desktop_test::fake_bus::{FakeBus, Reply, socket_path};
use serde_json::json;
use std::sync::atomic::AtomicBool;

fn task() -> (Task, Inbox, std_mpsc::Receiver<Delivery>) {
    let mut config = SdkConfig::new("review-retry.sock");
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

async fn answer(task: &mut Task, inbox: &mut Inbox) {
    let response = tokio::time::timeout(Duration::from_secs(3), inbox.internal.recv())
        .await
        .expect("answer deadline")
        .expect("response");
    task.internal(response);
}

fn apply(delivered: &std_mpsc::Receiver<Delivery>, store: &mut Store, seq: &mut u64) {
    for delivery in delivered.try_iter() {
        *seq += 1;
        store.apply(&EventEnvelope::new(*seq, delivery.at, delivery.event)).expect("event");
    }
}

#[tokio::test]
async fn recovered_reads_resume_timer_refresh_and_reset_failure_episode() {
    let path = socket_path("recovered-timers");
    let fake = FakeBus::start(&path, |_, _, _| Reply::Ok(json!({})));
    for fetch in
        [Fetch::DaemonStatus, Fetch::AccountList, Fetch::AccountStatus(AccountId::new("acct-main"))]
    {
        let (mut task, mut inbox, _) = task();
        task.client = BusClient::new(path.clone(), Duration::from_secs(2), task.config.deadlines);
        task.on_read(&fetch, Err(BusError::Timeout));
        let due = task.read_recovery.next_due().expect("first retry");
        let retry = task.read_recovery.due(due);
        task.spawn_reads(retry, false);
        answer(&mut task, &mut inbox).await;
        assert!(!task.read_recovery.blocks_timer(&fetch), "success ends the failed episode");
        let before = fake.seen().len();
        // This is also the networking re-read's spawn path in D2.
        task.spawn_reads(vec![fetch.clone()], false);
        answer(&mut task, &mut inbox).await;
        assert_eq!(fake.seen().len(), before + 1, "next timer refresh executes");
        task.on_read(&fetch, Err(BusError::Timeout));
        assert!(task.read_recovery.next_due().is_some(), "later failure gets its own retry");
    }
    // Exercise poll() itself after recovery, rather than only its predicate.
    let (mut task, mut inbox, _) = task();
    task.scope = Scope::Daemon;
    task.client = BusClient::new(path, Duration::from_secs(2), task.config.deadlines);
    task.on_read(&Fetch::AccountList, Err(BusError::Timeout));
    task.on_read(&Fetch::AccountList, Ok(json!({"accounts":[]})));
    let before = fake.seen().iter().filter(|r| r.method == "account.list").count();
    task.refresh_daemon();
    while !task.read_recovery.running.is_empty() {
        answer(&mut task, &mut inbox).await;
    }
    assert_eq!(fake.seen().iter().filter(|r| r.method == "account.list").count(), before + 1);
}

#[tokio::test]
async fn backup_permanent_refusal_sets_capability_and_finishes_listing() {
    for (code, capability) in [("CapabilityDenied", false), ("ValidationFailed", true)] {
        let path = socket_path("backup-refusal");
        let fake = FakeBus::start(&path, move |_, _, _| Reply::Error(code, "not available".into()));
        let (mut task, mut inbox, delivered) = task();
        task.client = BusClient::new(path, Duration::from_secs(2), task.config.deadlines);
        task.backup_answered(true);
        task.spawn_fetches(vec![Fetch::BackupJobs]);
        answer(&mut task, &mut inbox).await;
        let mut store = Store::new();
        let mut seq = 0;
        apply(&delivered, &mut store, &mut seq);
        assert_eq!(task.found.backup, capability, "{code} capability classification");
        assert!(store.is_listed(Listing::BackupJobs), "permanent refusal ends loading");
        assert!(store.failures().is_empty(), "specialized answer is not a generic failure");
        assert!(task.read_recovery.next_due().is_none(), "permanent answer is never retried");
        assert_eq!(fake.seen().len(), 1);
    }
}

#[tokio::test]
async fn in_flight_refresh_reads_committed_state_once_after_all_completion_paths() {
    for completion in 0..2 {
        for trigger in 0..3 {
            let path = socket_path("read-lost-update");
            let committed = Arc::new(AtomicBool::new(false));
            let state = committed.clone();
            let captured = Arc::new(AtomicBool::new(false));
            let read_captured = captured.clone();
            let fake = FakeBus::start(&path, move |_, _, _| {
                let requests = if state.load(Ordering::SeqCst) {
                    json!([])
                } else {
                    json!([{
                        "request_id":"old", "principal_id":"peer", "direction":"inbound", "state":"pending", "note":"old row"
                    }])
                };
                read_captured.store(true, Ordering::SeqCst);
                Reply::Slow(Duration::from_millis(40), json!({"requests":requests}))
            });
            let (mut task, mut inbox, delivered) = task();
            task.client = BusClient::new(path, Duration::from_secs(2), task.config.deadlines);
            let fetch = Fetch::ContactRequests;

            task.spawn_fetches(vec![fetch.clone()]);
            tokio::time::timeout(Duration::from_secs(3), async {
                while !captured.load(Ordering::SeqCst) {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("first request captured its old view");
            committed.store(true, Ordering::SeqCst);
            // Direct action/push refreshes and stream refreshes converge on one rerun.
            if trigger != 1 {
                task.spawn_fetches(vec![fetch.clone()]);
                task.spawn_fetches(vec![fetch.clone()]);
            }
            if trigger != 0 {
                task.request_reads(vec![fetch.clone()]);
                if trigger == 2 {
                    task.request_reads(vec![fetch.clone()]);
                }
            }
            {
                let first = tokio::time::timeout(Duration::from_secs(3), inbox.internal.recv())
                    .await
                    .expect("deadline")
                    .expect("first result");
                if completion == 1 {
                    let Internal::Fetched(read) = first else { panic!("fetch expected") };
                    task.internal(Internal::Snapshot(task.gate.epoch(), vec![read]));
                } else {
                    task.internal(first)
                }
            }
            let mut store = Store::new();
            let mut seq = 0;
            apply(&delivered, &mut store, &mut seq);
            assert_eq!(store.contact_requests().count(), 1, "old view applied first");
            assert!(
                task.read_recovery.running.contains_key(&fetch),
                "new trigger must survive the old answer"
            );
            answer(&mut task, &mut inbox).await;
            apply(&delivered, &mut store, &mut seq);
            assert_eq!(
                store.contact_requests().count(),
                0,
                "follow-up observes the committed acceptance"
            );
            assert!(!task.read_recovery.running.contains_key(&fetch));
            assert!(task.read_recovery.rerun.is_empty());
            assert_eq!(fake.seen().len(), 2, "exactly one follow-up");
            assert!(inbox.internal.try_recv().is_err(), "no third read or stuck completion");
        }
    }
}

#[tokio::test]
async fn recovery_reads_have_bounded_concurrency_and_drain_the_queue() {
    let path = socket_path("bounded-recovery");
    let fake = FakeBus::start(&path, |_, _, _| Reply::Ok(json!({"messages":[]})));
    let (mut task, mut inbox, _delivered) = task();
    task.client = BusClient::new(path, Duration::from_secs(2), task.config.deadlines);
    let reads: Vec<_> =
        (0..50).map(|n| Fetch::Messages(ContextId::new(format!("c-{n}")))).collect();
    task.request_reads(reads);
    assert_eq!(task.read_recovery.running.len(), 8);
    assert_eq!(task.read_recovery.queued.len(), 42);
    for _ in 0..50 {
        answer(&mut task, &mut inbox).await;
        assert!(task.read_recovery.running.len() <= 8);
    }
    assert!(task.read_recovery.running.is_empty());
    assert!(task.read_recovery.queued.is_empty());
    assert_eq!(fake.seen().len(), 50, "each queued read executes once");
    task.request_reads(
        (0..50).map(|n| Fetch::Messages(ContextId::new(format!("c-{n}")))).collect(),
    );
    task.locked();
    assert!(task.read_recovery.queued.is_empty(), "lock cancels queued reads too");
    assert!(task.read_recovery.running.is_empty());
}

#[tokio::test]
async fn focus_read_runs_next_ahead_of_fifo_recovery_queue() {
    let path = socket_path("focus-recovery-priority");
    let fake = FakeBus::start(&path, |_, _, _| Reply::Ok(json!({"messages":[]})));
    let (mut task, mut inbox, _delivered) = task();
    task.client = BusClient::new(path, Duration::from_secs(2), task.config.deadlines);
    let focus = ContextId::new("focus");
    task.translator.fetched(
        &Fetch::first(Paged::Conversations),
        &json!({"conversations":[{"id":"focus", "kind":"direct", "participants":[]}]}),
        now(),
    );
    let mut reads: Vec<_> =
        (0..MAX_READS).map(|n| Fetch::Messages(ContextId::new(format!("running-{n}")))).collect();
    // Deliberately reverse lexical order; focus is already in the recovery queue.
    reads.extend([
        Fetch::Messages(ContextId::new("z-background")),
        Fetch::Messages(focus.clone()),
        Fetch::Messages(ContextId::new("a-background")),
    ]);
    task.request_reads(reads);
    assert_eq!(task.read_recovery.running.len(), MAX_READS);
    task.command(ScopeCommand::Focus(Some(focus.clone())));
    answer(&mut task, &mut inbox).await;
    assert!(
        task.read_recovery.running.contains_key(&Fetch::Messages(focus)),
        "the first free slot belongs to the focused context"
    );
    assert!(task.read_recovery.queued.contains(&Fetch::Messages(ContextId::new("z-background"))));
    let background = Fetch::Messages(ContextId::new("z-background"));
    let later = Fetch::Messages(ContextId::new("a-background"));
    let mut saw_background = false;
    for _ in 1..MAX_READS + 5 {
        answer(&mut task, &mut inbox).await;
        assert!(task.read_recovery.running.len() <= MAX_READS);
        if !saw_background && task.read_recovery.running.contains_key(&background) {
            assert!(task.read_recovery.queued.contains(&later), "recovery starts in FIFO order");
            saw_background = true;
        }
    }
    assert!(saw_background);
    assert!(task.read_recovery.running.is_empty());
    assert!(task.read_recovery.queued.is_empty());
    let seen = fake.seen();
    assert_eq!(seen.len(), MAX_READS + 5, "focus coalesces the existing queued read");
    task.locked();
}
