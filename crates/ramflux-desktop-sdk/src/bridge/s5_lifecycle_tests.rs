// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Lifecycle recovery over the framed bus and the real desktop source.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ramflux_desktop_core::{
    AccountId, Effect, LockState, Networking, SecretInput, Source, Store, Timestamp, methods,
};
use ramflux_desktop_test::fake_bus::{FakeBus, Reply, socket_path};
use serde_json::{Value, json};

use crate::{SdkConfig, SdkHub, SdkSource};

fn until(source: &mut SdkSource, store: &mut Store, done: impl Fn(&Store) -> bool) {
    let limit = Instant::now() + Duration::from_secs(10);
    loop {
        for envelope in source.drain(Timestamp(0)) {
            let _ = store.apply(&envelope);
            if done(store) {
                return;
            }
        }
        if done(store) {
            return;
        }
        assert!(Instant::now() < limit, "desktop did not observe lifecycle completion");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn unlock() -> Effect {
    Effect::SecretCall {
        method: methods::ACCOUNT_UNLOCK.to_owned(),
        body: json!({"account_id": "acct-work"}),
        secrets: vec![SecretInput::new("test passphrase")],
    }
}

fn opened(store: &Store) -> bool {
    store
        .account(&AccountId::new("acct-work"))
        .is_some_and(|a| a.lock_state == LockState::Unlocked && a.networking == Networking::Pending)
}

fn exercise(name: &str, refuse_first: bool) {
    let path = socket_path(name);
    let calls = Arc::new(AtomicUsize::new(0));
    let applied = calls.clone();
    let frames: BTreeMap<String, Value> =
        serde_json::from_str(include_str!("../../tests/fixtures/rfd_frames.json")).expect("frames");
    let fake = FakeBus::start(&path, move |method, _, _| {
        if method == "account.unlock" {
            let attempt = applied.fetch_add(1, Ordering::SeqCst);
            if refuse_first && attempt == 0 {
                return Reply::Error(
                    "OperationReconciliationRequired",
                    "operation requires reconciliation: desk-op-1".into(),
                );
            }
            let body = json!({"local_account_id": "acct-work", "state": "unlocked", "networking": "networking_pending"});
            return if refuse_first { Reply::Ok(body) } else { Reply::Lost(body) };
        }
        // Reads always keep the account locked: only the returned lifecycle
        // body can satisfy the opened-account oracle below.
        let body = frames.get(method).cloned().unwrap_or_else(|| json!({}));
        Reply::Ok(body)
    });
    let mut config = SdkConfig::new(&path);
    config.poll_fast = Duration::from_secs(3600);
    config.poll_slow = Duration::from_secs(3600);
    let hub = SdkHub::start(config).expect("bridge");
    let mut source = hub.daemon_source();
    let mut store = Store::new();
    until(&mut source, &mut store, |s| s.account(&AccountId::new("acct-work")).is_some());
    assert!(!opened(&store));
    source.submit(unlock());
    if refuse_first {
        until(&mut source, &mut store, |s| !s.failures().is_empty());
        let failure = &store.failures()[0];
        assert_eq!(failure.message, crate::bus::RECONCILE_KEY);
        assert_eq!(
            fake.seen().iter().filter(|s| s.method == "account.unlock").count(),
            1,
            "a refusal is not automatically resent"
        );
        source.submit(unlock()); // Retry is a new user action.
    }
    until(&mut source, &mut store, opened);
    let sent: Vec<_> = fake.seen().into_iter().filter(|s| s.method == "account.unlock").collect();
    assert_eq!(sent.len(), 2);
    assert!(
        sent.iter().all(|s| s.operation_id.as_deref().is_some_and(|k| k.starts_with("desk-op-")))
    );
    assert_eq!(sent[0].body, sent[1].body);
    if refuse_first {
        assert_ne!(sent[0].operation_id, sent[1].operation_id);
        assert!(!sent[0].replayed && !sent[1].replayed);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    } else {
        assert_eq!(sent[0].operation_id, sent[1].operation_id);
        assert_ne!(sent[0].connection, sent[1].connection);
        assert!(!sent[0].replayed && sent[1].replayed);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the saved lifecycle body opens the account without reapplying unlock"
        );
        assert!(store.failures().is_empty());
    }
    fake.shutdown();
}

#[test]
fn reconcile_retry_sends_a_fresh_key() {
    exercise("s5-reconcile", true);
}

#[test]
fn lifecycle_answer_lost_resends_once_with_same_key() {
    exercise("s5-lifecycle", false);
}
