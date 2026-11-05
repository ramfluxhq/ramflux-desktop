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
    config.backup_api = false;
    let (out, delivered) = std_mpsc::channel();
    let (phrases, _) = std_mpsc::channel();
    let (task, inbox) = build(config, scope, out, Ceremonies { phrases, audit: Arc::default() });
    (task, inbox, delivered)
}

async fn pump(task: &mut Task, inbox: &mut Inbox, done: impl Fn(&Task) -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        while let Ok(read) = inbox.internal.try_recv() {
            task.internal(read);
        }
        while let Ok(event) = inbox.streamed.try_recv() {
            task.on_streamed(event);
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
    pump(task, inbox, |t| t.read_recovery.running.is_empty() && t.snapshot_task.is_none()).await;
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
    // Both the attach and stream-head recovery reads finished before this window.
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
    assert_eq!(rx.try_recv(), Ok(()), "every stream reopen refreshes daemon");
    account.on_streamed(Streamed::Event {
        seq: Some(1),
        at: None,
        topic: "account.state".into(),
        body: json!({}),
    });
    assert_eq!(rx.try_recv(), Ok(()), "account.state refreshes daemon");
}
