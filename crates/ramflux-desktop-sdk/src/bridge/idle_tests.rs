// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Actual framed sends during settled idle, failure backoff and action refreshes.
use super::*;
use crate::bus::stream::{StreamEnd, Streamed};
use ramflux_desktop_core::{AccountId, SyncMode};
use ramflux_desktop_test::fake_bus::{FakeBus, Reply, socket_path};
use serde_json::json;

fn task(path: &std::path::Path, scope: Scope) -> (Task, Inbox, std_mpsc::Receiver<Delivery>) {
    let mut config = SdkConfig::new(path);
    config.call_api = false;
    config.backup_api = true;
    let (out, delivered) = std_mpsc::channel();
    let (phrases, _) = std_mpsc::channel();
    let (task, inbox) =
        build(config, scope, out, Ceremonies { phrases: phrases.into(), audit: Arc::default() });
    (task, inbox, delivered)
}

async fn pump(task: &mut Task, inbox: &mut Inbox, done: impl Fn(&Task) -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        while let Ok(status) = inbox.connection_states.try_recv() {
            task.connection_changed(status);
        }
        while let Ok(read) = inbox.internal.try_recv() {
            task.internal(read);
        }
        while let Ok(event) = inbox.streamed.try_recv() {
            task.on_streamed(event);
        }
        while let Ok(event) = inbox.jobs.try_recv() {
            task.on_jobs(event);
        }
        task.tick();
        if done(task) {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "bridge settles");
        tokio::task::yield_now().await;
        std::thread::sleep(Duration::from_micros(100));
    }
}

async fn settle(task: &mut Task, inbox: &mut Inbox) {
    pump(task, inbox, |t| {
        t.read_recovery.running.is_empty()
            && t.snapshot_task.is_none()
            && t.jobs == super::super::jobs::Jobs::Open
    })
    .await;
}

fn data_count(fake: &FakeBus, mark: usize) -> usize {
    fake.seen()[mark..]
        .iter()
        .filter(|r| !matches!(r.method.as_str(), "bus.subscribe" | "subscription.open"))
        .count()
}

#[tokio::test(start_paused = true)]
async fn no_periodic_requests_when_idle() {
    let path = socket_path("settled-idle");
    let fake = FakeBus::start(&path, |_, _, _| Reply::Ok(json!({})));
    fake.enable_stream();
    let (mut task, mut inbox, _deliveries) =
        task(&path, Scope::Account(AccountId::new("acct-main")));
    task.snapshot();
    pump(&mut task, &mut inbox, |t| {
        t.sync.mode == SyncMode::Stream
            && t.read_recovery.running.is_empty()
            && t.snapshot_task.is_none()
    })
    .await;
    settle(&mut task, &mut inbox).await;
    // Attach, stream-head and job-open reads finished before this window.
    let mark = fake.seen().len();
    for _ in 0..600 {
        tokio::time::advance(Duration::from_secs(1)).await;
        task.tick();
        settle(&mut task, &mut inbox).await;
    }
    assert_eq!(data_count(&fake, mark), 0, "zero data requests over ten minutes");
    assert_eq!(fake.seen().len() - mark, 0, "zero subscription/setup/control requests");
}

#[tokio::test(start_paused = true)]
async fn failed_stream_only_reopens_subscription_without_data_polls() {
    let path = socket_path("failed-idle");
    let fake = FakeBus::start(&path, |_, _, _| Reply::Ok(json!({})));
    let (mut task, mut inbox, _deliveries) =
        task(&path, Scope::Account(AccountId::new("acct-main")));
    task.snapshot();
    pump(&mut task, &mut inbox, |t| t.sync.mode == SyncMode::Off && t.sync.open_at.is_some()).await;
    settle(&mut task, &mut inbox).await;
    let mark = fake.seen().len();
    for attempt in 1..=5 {
        tokio::time::advance(Duration::from_millis(1999)).await;
        task.tick();
        tokio::task::yield_now().await;
        assert_eq!(fake.seen().len() - mark, attempt - 1, "no attempt before backoff");
        tokio::time::advance(Duration::from_millis(1)).await;
        task.tick();
        pump(&mut task, &mut inbox, |t| t.sync.open_at.is_some()).await;
        assert_eq!(fake.seen().len() - mark, attempt);
        assert_eq!(data_count(&fake, mark), 0, "refusals never start data polls");
    }
    fake.enable_stream();
    tokio::time::advance(Duration::from_secs(2)).await;
    task.tick();
    pump(&mut task, &mut inbox, |t| {
        t.sync.mode == SyncMode::Stream && t.read_recovery.running.is_empty()
    })
    .await;
    // Recovery action reads settle, then restart the healthy observation window.
    let mark = fake.seen().len();
    tokio::time::advance(Duration::from_secs(600)).await;
    task.tick();
    settle(&mut task, &mut inbox).await;
    assert_eq!(data_count(&fake, mark), 0);
    assert_eq!(fake.seen().len() - mark, 0);
    // Connection loss follows the same two-second subscription-only policy.
    task.on_streamed(Streamed::Ended(StreamEnd::Lost(BusError::Timeout)));
    assert_eq!(task.sync.open_at, Some(Instant::now() + Duration::from_secs(2)));
}

#[tokio::test(start_paused = true)]
async fn daemon_scope_refreshes_on_actions_not_timers() {
    let path = socket_path("daemon-actions");
    let fake = FakeBus::start(&path, |_, _, _| Reply::Ok(json!({"accounts":[]})));
    let (mut daemon, mut inbox, _deliveries) = task(&path, Scope::Daemon);
    daemon.snapshot();
    settle(&mut daemon, &mut inbox).await;
    let mark = fake.seen().len();
    tokio::time::advance(Duration::from_secs(600)).await;
    daemon.tick();
    settle(&mut daemon, &mut inbox).await;
    assert_eq!(fake.seen().len(), mark);
    daemon.on_effect_done("account.unlock", &json!({}), Ok(json!({})), vec![]);
    settle(&mut daemon, &mut inbox).await;
    let methods: Vec<_> = fake.seen()[mark..].iter().map(|r| r.method.clone()).collect();
    assert_eq!(methods.len(), 2);
    assert!(methods.contains(&"daemon.status".into()) && methods.contains(&"account.list".into()));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (mut account, _, _delivered) = task(&path, Scope::Account(AccountId::new("acct-main")));
    account.daemon_refresh = Some(tx);
    account.on_streamed(Streamed::Opened { head_seq: 0, floor_seq: 0 });
    assert!(
        matches!(rx.try_recv(), Ok(super::super::DaemonNotice::Refresh)),
        "every stream reopen refreshes daemon"
    );
    account.on_streamed(Streamed::Event {
        seq: Some(1),
        at: None,
        topic: "account.state".into(),
        body: json!({}),
    });
    assert!(
        matches!(rx.try_recv(), Ok(super::super::DaemonNotice::Refresh)),
        "account.state refreshes daemon"
    );
}

#[tokio::test(start_paused = true)]
async fn ordinary_disconnect_is_observed_without_data_requests() {
    let path = socket_path("ordinary-idle-close");
    let sent = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = sent.clone();
    let fake = FakeBus::start(&path, move |_, _, _| {
        count.fetch_add(1, Ordering::SeqCst);
        Reply::Ok(json!({}))
    });
    let (mut daemon, mut inbox, delivered) = task(&path, Scope::Daemon);
    daemon.config.backup_api = false;
    daemon.snapshot();
    pump(&mut daemon, &mut inbox, |t| {
        t.link == Link::Live && t.snapshot_task.is_none() && t.read_recovery.running.is_empty()
    })
    .await;
    delivered.try_iter().for_each(drop);
    let before = sent.load(Ordering::SeqCst);
    fake.shutdown();
    pump(&mut daemon, &mut inbox, |t| t.transport_disconnected && t.link == Link::Lost).await;
    assert_eq!(daemon.link, Link::Lost, "ordinary EOF schedules independent recovery");
    assert!(
        delivered.try_iter().any(|d| matches!(
            d.event,
            DomainEvent::SourceStatusChanged(SourceStatus::Disconnected)
        ))
    );
    daemon.internal(Internal::Fetched((Fetch::DaemonStatus, None, Ok(json!({})))));
    assert!(
        !delivered
            .try_iter()
            .any(|d| matches!(d.event, DomainEvent::SourceStatusChanged(SourceStatus::Connected))),
        "late pre-EOF reply cannot restore connectivity"
    );
    for _ in 0..3 {
        tokio::time::advance(Duration::from_secs(2)).await;
        daemon.tick();
        pump(&mut daemon, &mut inbox, |t| t.link == Link::Lost).await;
        assert_eq!(sent.load(Ordering::SeqCst), before, "down daemon receives no frames");
    }
    let restarted = FakeBus::start(&path, |_, _, _| Reply::Ok(json!({})));
    tokio::time::advance(Duration::from_millis(1999)).await;
    daemon.tick();
    assert!(restarted.seen().is_empty(), "reconnect respects backoff");
    tokio::time::advance(Duration::from_millis(1)).await;
    daemon.tick();
    pump(&mut daemon, &mut inbox, |t| t.link == Link::Live && t.snapshot_task.is_none()).await;
    let methods: Vec<_> = restarted.seen().iter().map(|r| r.method.clone()).collect();
    assert_eq!(methods.len(), 3, "exactly one full daemon snapshot: {methods:?}");
    for method in ["daemon.status", "account.list", "node.list"] {
        assert!(methods.iter().any(|m| m == method), "snapshot includes {method}");
    }
    tokio::time::advance(Duration::from_secs(600)).await;
    daemon.tick();
    assert_eq!(restarted.seen().len(), 3, "no recurring snapshot");
}

#[tokio::test(start_paused = true)]
async fn daemon_eof_with_only_locked_account_reconnects_and_refreshes_nodes_without_jobs() {
    let path = socket_path("daemon-locked-restart");
    let mut frames: std::collections::BTreeMap<String, serde_json::Value> =
        serde_json::from_str(include_str!("../../tests/fixtures/rfd_frames.json")).expect("frames");
    frames.insert(
        "account.list".into(),
        json!({"accounts":[],"account_details":[{"id":"acct-main","label":"Me","state":"locked"}]}),
    );
    let sent = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = sent.clone();
    let first = frames.clone();
    let fake = FakeBus::start(&path, move |method, _, _| {
        observed.fetch_add(1, Ordering::SeqCst);
        Reply::Ok(first.get(method).cloned().unwrap_or(json!({})))
    });
    let (mut daemon, mut inbox, delivered) = task(&path, Scope::Daemon);
    daemon.config.backup_api = false;
    let (mut account, _account_inbox, _account_delivery) =
        task(&path, Scope::Account(AccountId::new("acct-main")));
    account.config.backup_api = false;
    account.locked();
    daemon.accounts_live(true);
    assert_eq!(account.link, Link::Locked);
    assert!(account.sync.open_at.is_none());
    daemon.snapshot();
    pump(&mut daemon, &mut inbox, |t| {
        t.link == Link::Live && t.snapshot_task.is_none() && t.read_recovery.running.is_empty()
    })
    .await;
    assert!(!daemon.found.backup);
    assert!(daemon.jobs_open_at.is_none());
    assert!(delivered.try_iter().any(|d| matches!(d.event, DomainEvent::NodeUpserted(_))));
    let count = sent.load(Ordering::SeqCst);
    fake.shutdown();
    pump(&mut daemon, &mut inbox, |t| t.transport_disconnected && t.link == Link::Lost).await;
    assert_eq!(daemon.link, Link::Lost);
    assert_eq!(daemon.reconnect_at, Instant::now() + Duration::from_secs(2));
    // Failed reconnects are bounded and send no frames while rfd is down.
    for _ in 0..2 {
        tokio::time::advance(Duration::from_millis(1999)).await;
        daemon.tick();
        assert_eq!(daemon.link, Link::Lost);
        tokio::time::advance(Duration::from_millis(1)).await;
        daemon.tick();
        pump(&mut daemon, &mut inbox, |t| t.link == Link::Lost && t.snapshot_task.is_none()).await;
        assert_eq!(sent.load(Ordering::SeqCst), count);
    }
    let mut second = frames;
    second.get_mut("node.list").expect("nodes")["nodes"][0]["node_id"] = json!("n-restarted");
    second.get_mut("node.list").expect("nodes")["default_node_id"] = json!("n-restarted");
    let restarted = FakeBus::start(&path, move |method, _, _| {
        Reply::Ok(second.get(method).cloned().unwrap_or(json!({})))
    });
    tokio::time::advance(Duration::from_millis(1999)).await;
    daemon.tick();
    assert!(restarted.seen().is_empty());
    tokio::time::advance(Duration::from_millis(1)).await;
    daemon.tick();
    pump(&mut daemon, &mut inbox, |t| {
        t.link == Link::Live && t.snapshot_task.is_none() && t.read_recovery.running.is_empty()
    })
    .await;
    assert!(delivered.try_iter().any(
        |d| matches!(d.event, DomainEvent::NodeUpserted(node) if node.id.as_str() == "n-restarted")
    ));
    let methods: Vec<_> = restarted.seen().iter().map(|r| r.method.clone()).collect();
    assert_eq!(methods.len(), 3, "exactly one full daemon snapshot");
    for method in ["daemon.status", "node.list", "account.list"] {
        assert_eq!(methods.iter().filter(|m| *m == method).count(), 1);
    }
    tokio::time::advance(Duration::from_secs(600)).await;
    daemon.tick();
    assert_eq!(restarted.seen().len(), 3, "settled reconnect does not poll");
}

#[tokio::test(start_paused = true)]
async fn stream_timeout_keeps_healthy_ordinary_connection_connected() {
    let path = socket_path("stream-only-timeout");
    let fake = FakeBus::start(&path, |_, _, _| Reply::Ok(json!({})));
    fake.enable_stream();
    let (mut task, mut inbox, delivered) = task(&path, Scope::Account(AccountId::new("acct-main")));
    task.snapshot();
    settle(&mut task, &mut inbox).await;
    delivered.try_iter().for_each(drop);
    task.on_streamed(Streamed::Ended(StreamEnd::Lost(BusError::Timeout)));
    task.on_jobs(JobStreamed::Ended(StreamEnd::Lost(BusError::Timeout)));
    assert!(!task.transport_disconnected);
    assert_eq!(task.link, Link::Live);
    assert!(
        !delivered.try_iter().any(|d| matches!(
            d.event,
            DomainEvent::SourceStatusChanged(SourceStatus::Disconnected)
        ))
    );
}
