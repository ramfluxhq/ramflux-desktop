// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A lock the daemon starts reaches the store without any focus change.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::bus::stream::{StreamEnd, Streamed};
use ramflux_desktop_core::{AccountId, AppLockState};
use ramflux_desktop_core::{Effect, methods};
use ramflux_desktop_test::fake_bus::{FakeBus, Reply, socket_path};
use serde_json::json;

fn task(path: &std::path::Path, scope: Scope) -> (Task, Inbox, std_mpsc::Receiver<Delivery>) {
    let mut config = SdkConfig::new(path);
    config.schema_support = crate::config::SchemaSupport::Legacy;
    config.call_api = false;
    config.backup_api = true;
    let (out, delivered) = std_mpsc::channel();
    let (phrases, _) = std_mpsc::channel();
    let (mut task, inbox) =
        build(config, scope, out, Ceremonies { phrases: phrases.into(), audit: Arc::default() });
    task.client = BusClient::new(path.to_path_buf(), Duration::from_secs(2), task.config.deadlines);
    task.link = Link::Live;
    task.open_gate_for_tests();
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

fn lock_reads(fake: &FakeBus) -> usize {
    fake.seen().iter().filter(|r| r.method == "account.app_lock.state").count()
}

fn app_lock_states(delivered: &std_mpsc::Receiver<Delivery>) -> Vec<AppLockState> {
    delivered
        .try_iter()
        .filter_map(|d| match d.event {
            DomainEvent::AppLockChanged { lock, .. } => Some(lock.state),
            _ => None,
        })
        .collect()
}

fn daemon_listing() -> serde_json::Value {
    json!({
        "accounts": ["acct-main"],
        "account_details": [{"id": "acct-main", "state": "unlocked", "label": "Main"}]
    })
}

fn lifecycle(state: &str) -> Effect {
    Effect::Call {
        method: methods::APP_LOCK_LIFECYCLE.to_owned(),
        body: json!({"account_id": "acct-main", "state": state}),
    }
}

/// The daemon session reads the lock at the time its own answer to the
/// background report said, with no focus change after it.
#[tokio::test(start_paused = true)]
async fn a_daemon_lock_is_read_when_it_is_due_without_a_focus_change() {
    let path = socket_path("app-lock-due");
    let closed = Arc::new(AtomicBool::new(false));
    let flag = closed.clone();
    let fake = FakeBus::start(&path, move |method, _, _| match method {
        "account.app_lock.lifecycle" => Reply::Ok(json!({
            "enabled": true, "timeout_ms": 5_000, "state": "armed",
            "reason": "authenticated", "remaining_ms": 5_000
        })),
        "account.app_lock.state" if flag.load(Ordering::SeqCst) => {
            Reply::Ok(json!({"enabled": true, "state": "locked", "reason": "timeout"}))
        }
        "account.app_lock.state" => Reply::Ok(json!({
            "enabled": true, "state": "armed", "remaining_ms": 100
        })),
        _ => Reply::Ok(json!({})),
    });
    let (mut task, mut inbox, delivered) = task(&path, Scope::Daemon);
    assert!(task.account().is_none(), "the daemon session has no account of its own");
    task.effect(lifecycle("background"));
    pump(&mut task, &mut inbox, |t| t.app_lock_watch.next_due().is_some()).await;
    assert_eq!(app_lock_states(&delivered).last(), Some(&AppLockState::Unlocked));
    let reads = lock_reads(&fake);

    // The daemon locks at its deadline; the window never changes focus.
    closed.store(true, Ordering::SeqCst);
    tokio::time::advance(Duration::from_millis(4_900)).await;
    task.tick();
    tokio::task::yield_now().await;
    assert_eq!(lock_reads(&fake), reads, "nothing is read before the lock is due");
    tokio::time::advance(Duration::from_millis(500)).await;
    pump(&mut task, &mut inbox, |t| {
        lock_reads(&fake) > reads && t.read_recovery.running.is_empty()
    })
    .await;
    assert_eq!(app_lock_states(&delivered).last(), Some(&AppLockState::Locked));
    assert!(task.app_lock_watch.next_due().is_none(), "a closed lock is not read again");
}

/// An open account with no deadline reports `unlocked` with 0 remaining: the
/// bridge schedules nothing and sends no reads of its own.
#[tokio::test(start_paused = true)]
async fn an_unlocked_account_without_a_deadline_is_never_polled() {
    let path = socket_path("app-lock-idle");
    let fake = FakeBus::start(&path, |method, _, _| match method {
        "account.app_lock.lifecycle" | "account.app_lock.state" => Reply::Ok(json!({
            "enabled": true, "state": "unlocked", "reason": "authenticated", "remaining_ms": 0
        })),
        _ => Reply::Ok(json!({})),
    });
    let (mut task, mut inbox, delivered) = task(&path, Scope::Daemon);
    task.effect(lifecycle("foreground"));
    let seen = std::cell::RefCell::new(Vec::new());
    pump(&mut task, &mut inbox, |_| {
        seen.borrow_mut().extend(app_lock_states(&delivered));
        !seen.borrow().is_empty()
    })
    .await;
    assert_eq!(seen.borrow().last(), Some(&AppLockState::Unlocked));
    assert!(task.app_lock_watch.next_due().is_none());
    tokio::time::advance(Duration::from_secs(10)).await;
    task.tick();
    tokio::task::yield_now().await;
    assert_eq!(lock_reads(&fake), 0);
}

/// The deadline of each account is kept apart: a second account's answer does
/// not move the first one's read.
#[tokio::test(start_paused = true)]
async fn every_account_keeps_its_own_read() {
    let path = socket_path("app-lock-two");
    let _fake = FakeBus::start(&path, |method, _, _| match method {
        "account.app_lock.lifecycle" => Reply::Ok(json!({
            "enabled": true, "state": "armed", "remaining_ms": 1_000
        })),
        _ => Reply::Ok(json!({})),
    });
    let (mut task, mut inbox, _delivered) = task(&path, Scope::Daemon);
    task.effect(lifecycle("background"));
    task.effect(Effect::Call {
        method: methods::APP_LOCK_LIFECYCLE.to_owned(),
        body: json!({"account_id": "acct-other", "state": "foreground"}),
    });
    pump(&mut task, &mut inbox, |t| t.app_lock_watch.next_due().is_some()).await;
    assert_eq!(task.app_lock_watch.take_due(Instant::now() + Duration::from_secs(2)).len(), 2);
}

/// A refusal in an account session sends the daemon session to list and read
/// the locks again; its store then holds the lock the page is routed from.
#[tokio::test(start_paused = true)]
async fn an_app_locked_refusal_in_an_account_session_closes_the_daemon_store() {
    let path = socket_path("app-lock-refusal");
    let _fake = FakeBus::start(&path, |method, _, _| match method {
        "account.list" => Reply::Ok(daemon_listing()),
        "account.app_lock.state" => {
            Reply::Ok(json!({"enabled": true, "state": "locked", "reason": "timeout"}))
        }
        "account.status" | "message.list" => Reply::Error("app_locked", "locked".into()),
        _ => Reply::Ok(json!({})),
    });
    let (mut account, mut account_inbox, _account_events) =
        task(&path, Scope::Account(AccountId::new("acct-main")));
    let (notices_tx, mut notices) = tokio::sync::mpsc::unbounded_channel();
    account.daemon_refresh = Some(notices_tx);
    account.spawn_fetches(vec![Fetch::AccountStatus(AccountId::new("acct-main"))]);
    pump(&mut account, &mut account_inbox, |t| t.read_recovery.running.is_empty()).await;
    assert!(
        matches!(notices.try_recv(), Ok(crate::bridge::DaemonNotice::Refresh)),
        "the refusal refreshes the daemon session"
    );

    let (mut daemon, mut daemon_inbox, delivered) = task(&path, Scope::Daemon);
    daemon.refresh_daemon();
    let seen = std::cell::RefCell::new(Vec::new());
    pump(&mut daemon, &mut daemon_inbox, |t| {
        seen.borrow_mut().extend(app_lock_states(&delivered));
        t.read_recovery.running.is_empty() && !seen.borrow().is_empty()
    })
    .await;
    assert_eq!(seen.borrow().last(), Some(&AppLockState::Locked));
}

#[tokio::test(start_paused = true)]
async fn an_app_locked_refusal_of_the_stream_refreshes_the_daemon() {
    let path = socket_path("app-lock-stream");
    let _fake = FakeBus::start(&path, |_, _, _| Reply::Ok(json!({})));
    let (mut task, _inbox, delivered) = task(&path, Scope::Account(AccountId::new("acct-main")));
    let (tx, mut notices) = tokio::sync::mpsc::unbounded_channel();
    task.daemon_refresh = Some(tx);
    task.on_streamed(Streamed::Ended(StreamEnd::AppLocked));
    assert_eq!(app_lock_states(&delivered), vec![AppLockState::Locked]);
    assert_eq!(task.sync.open_at, Some(Instant::now() + task.config.reconnect_backoff));
    assert!(matches!(notices.try_recv(), Ok(crate::bridge::DaemonNotice::Refresh)));
}

#[tokio::test]
async fn a_daemon_session_refusal_names_the_account_of_its_request() {
    let path = socket_path("app-lock-effect");
    let _fake = FakeBus::start(&path, |method, _, _| match method {
        "account.app_lock.configure" => Reply::Error("app_locked", "locked".into()),
        _ => Reply::Ok(json!({})),
    });
    let (mut task, mut inbox, delivered) = task(&path, Scope::Daemon);
    task.effect(Effect::Call {
        method: methods::APP_LOCK_CONFIGURE.to_owned(),
        body: json!({"account_id": "acct-other", "enabled": true, "timeout_ms": 1_000}),
    });
    let response = inbox.internal.recv().await.expect("answer");
    task.internal(response);
    let locked: Vec<_> = delivered
        .try_iter()
        .filter_map(|d| match d.event {
            DomainEvent::AppLockChanged { account_id, lock } => Some((account_id, lock.state)),
            _ => None,
        })
        .collect();
    assert_eq!(locked, vec![(AccountId::new("acct-other"), AppLockState::Locked)]);
}

fn locked_event() -> serde_json::Value {
    json!({"state": "locked", "reason": "manual", "generation": 4, "remaining_ms": 0})
}

/// Another client locked the account while this window was idle: the account
/// stream carries the event, the lock page is routed at once, and the lock
/// and the daemon's listing are read again.
#[tokio::test(start_paused = true)]
async fn a_changed_event_on_the_stream_locks_the_account_at_once() {
    let path = socket_path("app-lock-changed-stream");
    let fake = FakeBus::start(&path, |method, _, _| match method {
        "account.app_lock.state" => {
            Reply::Ok(json!({"enabled": true, "state": "locked", "reason": "manual"}))
        }
        _ => Reply::Ok(json!({})),
    });
    let (mut task, mut inbox, delivered) = task(&path, Scope::Account(AccountId::new("acct-main")));
    let (tx, mut notices) = tokio::sync::mpsc::unbounded_channel();
    task.daemon_refresh = Some(tx);
    task.on_streamed(Streamed::Event {
        seq: None,
        at: None,
        topic: "account.app_lock.changed".to_owned(),
        body: locked_event(),
    });
    assert_eq!(app_lock_states(&delivered), vec![AppLockState::Locked], "applied before any read");
    assert!(matches!(notices.try_recv(), Ok(crate::bridge::DaemonNotice::Refresh)));
    pump(&mut task, &mut inbox, |t| t.read_recovery.running.is_empty()).await;
    assert_eq!(lock_reads(&fake), 1, "the lock is read again");
}

/// An unlock by another client is read, not assumed, and does not lock.
#[tokio::test(start_paused = true)]
async fn a_changed_event_that_is_not_locked_only_reads_the_lock() {
    let path = socket_path("app-lock-changed-open");
    let fake = FakeBus::start(&path, |_, _, _| Reply::Ok(json!({})));
    let (mut task, mut inbox, delivered) = task(&path, Scope::Account(AccountId::new("acct-main")));
    let (tx, _notices) = tokio::sync::mpsc::unbounded_channel();
    task.daemon_refresh = Some(tx);
    task.app_lock_changed(None, &json!({"state": "unlocked", "remaining_ms": 0}));
    pump(&mut task, &mut inbox, |t| t.read_recovery.running.is_empty()).await;
    assert_eq!(lock_reads(&fake), 1);
    assert_eq!(app_lock_states(&delivered), vec![AppLockState::Unlocked]);
}

/// An event that names another account is not this session's.
#[tokio::test(start_paused = true)]
async fn a_changed_event_of_another_account_is_ignored_by_an_account_session() {
    let path = socket_path("app-lock-changed-other");
    let fake = FakeBus::start(&path, |_, _, _| Reply::Ok(json!({})));
    let (mut task, _inbox, delivered) = task(&path, Scope::Account(AccountId::new("acct-main")));
    task.app_lock_changed(Some("acct-other"), &locked_event());
    assert!(app_lock_states(&delivered).is_empty());
    assert_eq!(lock_reads(&fake), 0);
}

/// With no account stream attached, the daemon session's request connection
/// is told: its store closes and it lists the accounts again.
#[tokio::test(start_paused = true)]
async fn a_changed_event_on_the_daemon_request_connection_locks_the_daemon_store() {
    let path = socket_path("app-lock-changed-daemon");
    let fake = FakeBus::start(&path, |method, _, _| match method {
        "account.list" => Reply::Ok(daemon_listing()),
        "account.app_lock.state" => {
            Reply::Ok(json!({"enabled": true, "state": "locked", "reason": "manual"}))
        }
        _ => Reply::Ok(json!({})),
    });
    let (mut task, mut inbox, delivered) = task(&path, Scope::Daemon);
    let (events, mut locks) = tokio::sync::mpsc::unbounded_channel();
    task.client = BusClient::with_status(
        path.clone(),
        Duration::from_secs(2),
        task.config.deadlines,
        None,
        Some(events),
        None,
    );
    task.client.attach().await.expect("connects");
    // The fake may not have registered the connection yet.
    let mut event = None;
    for _ in 0..200 {
        fake.push_to_connections("acct-main", "account.app_lock.changed", locked_event());
        tokio::task::yield_now().await;
        std::thread::sleep(Duration::from_millis(5));
        if let Ok(received) = locks.try_recv() {
            event = Some(received);
            break;
        }
    }
    let event = event.expect("the request connection forwards the event");
    task.app_lock_changed(event.account.as_deref(), &event.body);
    assert_eq!(app_lock_states(&delivered), vec![AppLockState::Locked], "applied at once");
    pump(&mut task, &mut inbox, |t| t.read_recovery.running.is_empty()).await;
    assert!(fake.seen().iter().any(|r| r.method == "account.list"), "the accounts are listed");
    assert!(lock_reads(&fake) >= 1);
}

#[test]
fn the_locked_stream_retry_backs_off_to_the_cap() {
    let base = Duration::from_secs(2);
    let waits: Vec<_> = (0..8).map(|n| crate::bridge::app_lock::stream_backoff(base, n)).collect();
    assert_eq!(waits[0], base);
    assert_eq!(waits[1], Duration::from_secs(4));
    assert_eq!(waits[3], Duration::from_secs(16));
    assert_eq!(waits[4], Duration::from_secs(30));
    assert_eq!(waits[7], Duration::from_secs(30));
}

#[tokio::test(start_paused = true)]
async fn repeated_locked_refusals_of_the_stream_retry_later_each_time() {
    let path = socket_path("app-lock-stream-backoff");
    let _fake = FakeBus::start(&path, |_, _, _| Reply::Ok(json!({})));
    let (mut task, _inbox, _delivered) = task(&path, Scope::Account(AccountId::new("acct-main")));
    let base = task.config.reconnect_backoff;
    let mut waits = Vec::new();
    for _ in 0..3 {
        task.on_streamed(Streamed::Ended(StreamEnd::AppLocked));
        waits.push(task.sync.open_at.map(|at| at - Instant::now()));
    }
    assert_eq!(waits, vec![Some(base), Some(base * 2), Some(base * 4)]);
}

/// An account session whose status read says the account is not open sends
/// the daemon session to list the accounts again.
#[tokio::test(start_paused = true)]
async fn an_account_not_open_answer_refreshes_the_daemon() {
    let path = socket_path("app-lock-not-open");
    let _fake = FakeBus::start(&path, |method, _, _| match method {
        "account.status" => Reply::Error("ValidationFailed", "account not open: acct-main".into()),
        _ => Reply::Ok(json!({})),
    });
    let (mut task, mut inbox, _delivered) =
        task(&path, Scope::Account(AccountId::new("acct-main")));
    let (tx, mut notices) = tokio::sync::mpsc::unbounded_channel();
    task.daemon_refresh = Some(tx);
    task.spawn_fetches(vec![Fetch::AccountStatus(AccountId::new("acct-main"))]);
    pump(&mut task, &mut inbox, |t| t.read_recovery.running.is_empty() && t.link == Link::Locked)
        .await;
    assert!(matches!(notices.try_recv(), Ok(crate::bridge::DaemonNotice::Refresh)));
}
