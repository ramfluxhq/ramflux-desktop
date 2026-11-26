// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A lock the daemon starts reaches the store without any focus change.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::bus::stream::{StreamEnd, Streamed};
use ramflux_desktop_core::{AccountId, AppLockState};
use ramflux_desktop_test::fake_bus::{FakeBus, Reply, socket_path};
use serde_json::json;

fn task(path: &std::path::Path) -> (Task, Inbox, std_mpsc::Receiver<Delivery>) {
    let mut config = SdkConfig::new(path);
    config.schema_support = crate::config::SchemaSupport::Legacy;
    config.call_api = false;
    config.backup_api = true;
    let (out, delivered) = std_mpsc::channel();
    let (phrases, _) = std_mpsc::channel();
    let (task, inbox) = build(
        config,
        Scope::Account(AccountId::new("acct-main")),
        out,
        Ceremonies { phrases: phrases.into(), audit: Arc::default() },
    );
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

#[tokio::test(start_paused = true)]
async fn a_daemon_lock_is_read_when_it_is_due_without_a_focus_change() {
    let path = socket_path("app-lock-due");
    let closed = Arc::new(AtomicBool::new(false));
    let flag = closed.clone();
    let fake = FakeBus::start(&path, move |method, _, _| {
        if method != "account.app_lock.state" {
            return Reply::Ok(json!({}));
        }
        if flag.load(Ordering::SeqCst) {
            Reply::Ok(json!({"enabled": true, "state": "locked", "reason": "timeout"}))
        } else {
            Reply::Ok(json!({
                "enabled": true, "state": "unlocked", "reason": "authenticated",
                "remaining_ms": 5_000
            }))
        }
    });
    fake.enable_stream();
    let (mut task, mut inbox, delivered) = task(&path);
    task.snapshot();
    pump(&mut task, &mut inbox, |t| {
        t.read_recovery.running.is_empty() && t.snapshot_task.is_none() && t.link == Link::Live
    })
    .await;
    task.spawn_fetches(vec![Fetch::AppLock(AccountId::new("acct-main"))]);
    pump(&mut task, &mut inbox, |t| {
        t.read_recovery.running.is_empty() && t.app_lock_watch.next_due().is_some()
    })
    .await;
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
}

#[tokio::test(start_paused = true)]
async fn an_app_locked_refusal_of_the_stream_closes_the_store() {
    let path = socket_path("app-lock-stream");
    let _fake = FakeBus::start(&path, |_, _, _| Reply::Ok(json!({})));
    let (mut task, _inbox, delivered) = task(&path);
    task.link = Link::Live;
    task.on_streamed(Streamed::Ended(StreamEnd::AppLocked));
    assert_eq!(app_lock_states(&delivered), vec![AppLockState::Locked]);
    assert_eq!(task.sync.open_at, Some(Instant::now() + task.config.reconnect_backoff));
}
