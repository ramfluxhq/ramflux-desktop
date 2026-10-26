// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The account source on the daemon's event stream (`bus.subscribe`), with
//! a fake daemon that keeps a journal as the daemon does: each topic calls
//! for its reads, nothing is polled while the stream is live, duplicates
//! and out-of-order entries change nothing, a cursor below the floor
//! reloads, a stopped stream resumes from the cursor, a lock stops it and
//! the next source resumes from the saved cursor, and a daemon without the
//! stream is polled (or, with the fallback off, not).
//!
//! Polling is set to 80 ms here, so "nothing was polled" is meaningful.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use common::*;
use ramflux_desktop_core::{AccountId, ContextId, DaemonSyncState, LockState, SyncMode};
use ramflux_desktop_sdk::SdkConfig;
use ramflux_desktop_sdk::plan::fetch::Fetch;
use ramflux_desktop_test::fake_bus::Seen;
use serde_json::{Value, json};

const ACCOUNT: &str = "acct-main";

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

/// A fake daemon (stream on unless `stream` is false) whose conversation
/// directory lists Zoe's new conversation once `listed` is set.
struct World {
    fake: FakeBus,
    listed: Arc<AtomicBool>,
    /// The account is locked: `account.status` answers as the daemon does.
    locked: Arc<AtomicBool>,
}

impl World {
    fn start(name: &str, stream: bool) -> (Self, PathBuf) {
        let path = socket_path(&format!("es-{name}"));
        let listed = Arc::new(AtomicBool::new(false));
        let flag = listed.clone();
        let locked = Arc::new(AtomicBool::new(false));
        let lock = locked.clone();
        let fake = FakeBus::start(&path, move |method, _, _| match method {
            "conversation.directory" => Reply::Ok(conversations(flag.load(Ordering::SeqCst))),
            "account.status" if lock.load(Ordering::SeqCst) => Reply::Error(
                "ValidationFailed",
                format!("local bus error: account not open: {ACCOUNT}"),
            ),
            _ => fixture(method),
        });
        if stream {
            fake.enable_stream();
        }
        (Self { fake, listed, locked }, path)
    }

    fn mark(&self) -> usize {
        self.fake.seen().len()
    }

    /// Requests of `method` received after `mark`.
    fn since(&self, mark: usize, method: &str) -> Vec<Seen> {
        self.fake.seen().into_iter().skip(mark).filter(|s| s.method == method).collect()
    }

    /// The account's `bus.subscribe` requests (not the daemon-level job
    /// stream's, which carry no account).
    fn subscribes(&self) -> Vec<Seen> {
        self.since(0, "bus.subscribe").into_iter().filter(|s| s.account.is_some()).collect()
    }

    /// Whether every read of the account snapshot reached the fake after
    /// the last `bus.subscribe`: the reload a scope does when its stream
    /// opens from the head.
    fn reloaded(&self) -> bool {
        let seen = self.fake.seen();
        let Some(subscribed) =
            seen.iter().rposition(|s| s.method == "bus.subscribe" && s.account.is_some())
        else {
            return false;
        };
        let after = &seen[subscribed + 1..];
        Fetch::account_snapshot(&AccountId::new(ACCOUNT))
            .iter()
            .map(|fetch| fetch.request(Some(ACCOUNT)).method)
            .all(|method| after.iter().any(|s| s.method == method))
    }
}

fn settings_file(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rf-es-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("desktop-settings.json")
}

fn config_with(path: &std::path::Path, settings: Option<PathBuf>) -> SdkConfig {
    SdkConfig { settings_path: settings, ..config(path) }
}

/// An account source fed into a store, waited on until its stream is open.
fn streaming(world: &World, config: SdkConfig) -> Fed {
    let hub = ramflux_desktop_sdk::SdkHub::start(config).expect("bridge");
    let mut account = Fed::new(hub.account_source(AccountId::new(ACCOUNT)));
    account.until("the stream is open", |s, _| {
        s.context(&ContextId::new("dm-ana")).is_some()
            && s.sync_status().is_some_and(|sync| sync.mode == SyncMode::Stream)
    });
    assert!(world.fake.streaming(ACCOUNT));
    // Opened from the head (no cursor), the scope reads everything again so
    // that nothing between its snapshot and the subscription is missed. The
    // stream shows as open before those reads reach the daemon; wait for
    // them, so the requests a test counts afterwards are its own.
    let opened = world.subscribes().pop().expect("subscribed");
    if opened.body.get("since_seq").is_none() {
        account.until("the reload on open", |_, _| world.reloaded());
    }
    account
}

/// Waits until the fake received `method` after `mark`.
fn wait_for(world: &World, account: &mut Fed, mark: usize, method: &str) {
    account.until(method, |_, _| !world.since(mark, method).is_empty());
}

#[test]
fn every_topic_calls_for_its_reads_and_nothing_is_polled() {
    let (world, path) = World::start("topics", true);
    let mut account = streaming(&world, config(&path));
    // Live: the poll interval passes several times; nothing is read.
    let quiet = world.mark();
    account.settle(500);
    let polled: Vec<String> = world.fake.seen().into_iter().skip(quiet).map(|s| s.method).collect();
    assert!(polled.is_empty(), "nothing is polled while the stream is live: {polled:?}");

    let cases: [(&str, Value, &[&str]); 11] = [
        ("conversation.changed", json!({ "id": "dm-ana" }), &["conversation.directory"]),
        (
            "message.received",
            json!({ "message": { "id": "m3", "conversation_id": "dm-ana" },
                    "conversation": { "id": "dm-ana" } }),
            &["message.list", "conversation.directory"],
        ),
        (
            "message.updated",
            json!({ "message_id": "m2", "conversation_id": "dm-ana" }),
            &["message.list"],
        ),
        (
            "group.message.received",
            json!({ "message": { "id": "g1", "conversation_id": "dm-unknown" },
                    "conversation": { "id": "dm-unknown" } }),
            &["conversation.directory"],
        ),
        ("group.changed", json!({ "id": "g-team" }), &["group.list"]),
        (
            "contact.changed",
            json!({ "link_id": "l", "requester_id": "a", "target_id": "b" }),
            &["contact.list", "identity.directory"],
        ),
        ("safety.changed", json!({ "id": "principal-ana" }), &["identity.directory"]),
        ("object.changed", json!({ "id": "attachment:m1:0" }), &["object.directory"]),
        ("object.transfer.progress", json!({ "id": "attachment:m1:0" }), &["object.directory"]),
        ("mcp.approval.request", json!({ "approval_id": "appr-9" }), &["mcp.approval.list"]),
        ("message.unrouted", json!({ "envelope_id": "e" }), &["conversation.directory"]),
    ];
    for (topic, body, reads) in cases {
        let mark = world.mark();
        world.fake.publish(ACCOUNT, topic, body);
        for read in reads {
            wait_for(&world, &mut account, mark, read);
        }
        assert!(world.since(mark, "message.receive").is_empty(), "{topic}: no pull from the node");
    }
    drop(account);
    world.fake.shutdown();
}

#[test]
fn a_first_message_from_an_unknown_conversation_appears_through_conversation_changed() {
    let (world, path) = World::start("first-dm", true);
    let mut account = streaming(&world, config(&path));
    let new = ContextId::new("dm-from-zoe");
    world.listed.store(true, Ordering::SeqCst);
    let row = json!({ "id": "dm-from-zoe", "kind": "direct", "participants": ["principal-zoe"] });
    world.fake.publish(ACCOUNT, "conversation.changed", row);
    account.until("the new conversation", |s, _| s.context(&new).is_some());
    drop(account);
    world.fake.shutdown();
}

#[test]
fn state_topics_are_applied_directly() {
    let (world, path) = World::start("state", true);
    let mut account = streaming(&world, config(&path));
    let seq = world.fake.publish(
        ACCOUNT,
        "sync.status",
        json!({ "state": "running", "last_run": 1_790_000_100, "unrouted": 1 }),
    );
    account.until("the daemon's sync", |s, _| {
        s.sync_status().is_some_and(|sync| {
            sync.daemon_sync == Some(DaemonSyncState::Running)
                && sync.unrouted == Some(1)
                && sync.cursor == Some(seq)
        })
    });
    world.fake.publish(
        ACCOUNT,
        "account.state",
        json!({ "local_account_id": "other", "state": "locked" }),
    );
    account.settle(200);
    let me = AccountId::new(ACCOUNT);
    assert_ne!(account.store.account(&me).map(|a| a.lock_state), Some(LockState::Locked));
    drop(account);
    world.fake.shutdown();
}

#[test]
fn duplicate_and_out_of_order_entries_change_nothing() {
    let (world, path) = World::start("dupes", true);
    let mut account = streaming(&world, config(&path));
    let mark = world.mark();
    let seq = world.fake.publish(ACCOUNT, "conversation.changed", json!({ "id": "dm-ana" }));
    wait_for(&world, &mut account, mark, "conversation.directory");
    account.settle(200);
    let after = world.mark();
    // The same entry again, and an older one: both are below the cursor.
    world.fake.deliver_raw(ACCOUNT, Some(seq), "conversation.changed", &json!({ "id": "dm-ana" }));
    world.fake.deliver_raw(ACCOUNT, Some(seq - 1), "object.changed", &json!({ "id": "o" }));
    account.settle(400);
    let reads: Vec<String> = world.fake.seen().into_iter().skip(after).map(|s| s.method).collect();
    assert!(reads.is_empty(), "duplicates call for no reads: {reads:?}");
    // A newer entry still applies.
    let mark = world.mark();
    world.fake.deliver_raw(ACCOUNT, Some(seq + 5), "object.changed", &json!({ "id": "o" }));
    wait_for(&world, &mut account, mark, "object.directory");
    drop(account);
    world.fake.shutdown();
}

#[test]
fn a_cursor_below_the_floor_reloads_and_follows_from_the_head() {
    let (world, path) = World::start("floor", true);
    let settings = settings_file("floor");
    for _ in 0..3 {
        world.fake.publish(ACCOUNT, "object.changed", json!({ "id": "o" }));
    }
    ramflux_desktop_sdk::cursor::CursorStore::new(Some(settings.clone()), path.to_string_lossy())
        .save(&AccountId::new(ACCOUNT), 1);
    // Entries 1..5 are gone; the daemon keeps 6 onwards.
    world.fake.prune_below(ACCOUNT, 6);
    world.fake.publish(ACCOUNT, "object.changed", json!({ "id": "o" }));
    world.listed.store(true, Ordering::SeqCst);
    let mut account = streaming(&world, config_with(&path, Some(settings.clone())));
    let subscribes = world.subscribes();
    assert_eq!(subscribes[0].body["since_seq"], 1, "resumes from the saved cursor");
    let last = subscribes.last().expect("resubscribed");
    assert!(last.body.get("since_seq").is_none(), "follows from the head: {:?}", last.body);
    // The reload reads everything again: the conversation that appeared
    // while the cursor was out of range is there.
    account.until("the reload", |s, _| s.context(&ContextId::new("dm-from-zoe")).is_some());
    let head = world.fake.head(ACCOUNT);
    account.until("cursor at head", |s, _| s.sync_status().and_then(|x| x.cursor) == Some(head));
    drop(account);
    world.fake.shutdown();
    let _ = std::fs::remove_dir_all(settings.parent().expect("dir"));
}

#[test]
fn a_stopped_stream_resumes_from_the_cursor_and_misses_nothing() {
    for lagged in [true, false] {
        let (world, path) = World::start(if lagged { "lag" } else { "close" }, true);
        let mut account = streaming(&world, config(&path));
        let mark = world.mark();
        let seq = world.fake.publish(ACCOUNT, "object.changed", json!({ "id": "o" }));
        wait_for(&world, &mut account, mark, "object.directory");
        world.fake.end_streams(ACCOUNT, lagged);
        // Published while the stream is down: replayed on resume.
        world.listed.store(true, Ordering::SeqCst);
        world.fake.publish(ACCOUNT, "conversation.changed", json!({ "id": "dm-from-zoe" }));
        account.until("resumed", |s, _| s.context(&ContextId::new("dm-from-zoe")).is_some());
        let subscribes = world.subscribes();
        assert!(subscribes.len() >= 2, "resubscribed");
        assert_eq!(subscribes[1].body["since_seq"], seq, "from the last applied entry");
        let status = account.store.sync_status().cloned().expect("status");
        assert_eq!(status.mode, SyncMode::Stream);
        drop(account);
        world.fake.shutdown();
    }
}

#[test]
fn a_lock_stops_the_stream_and_the_next_source_resumes_from_the_cursor() {
    let (world, path) = World::start("lock", true);
    let settings = settings_file("lock");
    let config = config_with(&path, Some(settings.clone()));
    let mut account = streaming(&world, config.clone());
    let mark = world.mark();
    world.fake.publish(ACCOUNT, "object.changed", json!({ "id": "o" }));
    wait_for(&world, &mut account, mark, "object.directory");
    world.locked.store(true, Ordering::SeqCst);
    let locked = world.fake.publish(
        ACCOUNT,
        "account.state",
        json!({ "local_account_id": ACCOUNT, "state": "locked" }),
    );
    let me = AccountId::new(ACCOUNT);
    account
        .until("locked", |s, _| s.account(&me).is_some_and(|a| a.lock_state == LockState::Locked));
    let deadline = Instant::now() + Duration::from_secs(10);
    while world.fake.streaming(ACCOUNT) {
        assert!(Instant::now() < deadline, "the stream stops on lock");
        std::thread::sleep(Duration::from_millis(20));
    }
    let saved = ramflux_desktop_sdk::cursor::CursorStore::new(
        Some(settings.clone()),
        path.to_string_lossy(),
    )
    .load(&me);
    assert_eq!(saved, Some(locked), "the cursor is saved on lock");
    drop(account);
    // Unlock: the app makes a new source for the account.
    world.locked.store(false, Ordering::SeqCst);
    let before = world.subscribes().len();
    let hub = ramflux_desktop_sdk::SdkHub::start(config).expect("bridge");
    let mut again = Fed::new(hub.account_source(me));
    again.until("resubscribed", |_, _| world.subscribes().len() > before);
    assert_eq!(world.subscribes()[before].body["since_seq"], locked);
    drop(again);
    world.fake.shutdown();
    let _ = std::fs::remove_dir_all(settings.parent().expect("dir"));
}

#[test]
fn a_replayed_lock_of_an_account_unlocked_since_does_not_lock_it() {
    // The desktop locked the account itself, so its stream stopped before
    // the daemon's `locked` entry; after the unlock the new source resumes
    // from the older cursor and is replayed that entry (then `unlocked`).
    let (world, path) = World::start("relock", true);
    let before = world.fake.publish(ACCOUNT, "object.changed", json!({ "id": "o" }));
    world.fake.publish(
        ACCOUNT,
        "account.state",
        json!({ "local_account_id": ACCOUNT, "state": "locked" }),
    );
    world.fake.publish(
        ACCOUNT,
        "account.state",
        json!({ "local_account_id": ACCOUNT, "state": "unlocked" }),
    );
    let settings = settings_file("relock");
    ramflux_desktop_sdk::cursor::CursorStore::new(Some(settings.clone()), path.to_string_lossy())
        .save(&AccountId::new(ACCOUNT), before);
    let mut account = streaming(&world, config_with(&path, Some(settings.clone())));
    assert_eq!(world.subscribes()[0].body["since_seq"], before, "the lock entry is replayed");
    account.settle(500);
    let me = AccountId::new(ACCOUNT);
    assert_ne!(
        account.store.account(&me).map(|a| a.lock_state),
        Some(LockState::Locked),
        "the daemon says the account is open"
    );
    // The stream stays open: a later change still arrives.
    let mark = world.mark();
    world.fake.publish(ACCOUNT, "object.changed", json!({ "id": "o" }));
    wait_for(&world, &mut account, mark, "object.directory");
    drop(account);
    world.fake.shutdown();
    let _ = std::fs::remove_dir_all(settings.parent().expect("dir"));
}

#[test]
fn without_the_stream_the_source_polls_and_says_so() {
    let (world, path) = World::start("fallback", false);
    let hub = ramflux_desktop_sdk::SdkHub::start(config(&path)).expect("bridge");
    let mut account = Fed::new(hub.account_source(AccountId::new(ACCOUNT)));
    account.until("polling", |s, _| {
        s.sync_status().is_some_and(|sync| sync.mode == SyncMode::Polling)
    });
    let reason = account.store.sync_status().and_then(|s| s.reason.clone()).expect("reason");
    assert!(reason.contains("bus.subscribe"), "{reason}");
    account.until("the push subscription", |_, _| world.fake.subscribed(ACCOUNT));
    let mark = world.mark();
    account.until("a poll", |_, _| world.since(mark, "conversation.directory").len() >= 2);
    drop(account);
    world.fake.shutdown();
}

#[test]
fn with_the_fallback_off_nothing_is_polled() {
    let (world, path) = World::start("off", false);
    let config = SdkConfig { poll_fallback: false, ..config(&path) };
    let hub = ramflux_desktop_sdk::SdkHub::start(config).expect("bridge");
    let mut account = Fed::new(hub.account_source(AccountId::new(ACCOUNT)));
    account.until("off", |s, _| s.sync_status().is_some_and(|sync| sync.mode == SyncMode::Off));
    let mark = world.mark();
    account.settle(500);
    let polled: Vec<String> = world.fake.seen().into_iter().skip(mark).map(|s| s.method).collect();
    assert!(polled.is_empty(), "nothing is polled with the fallback off: {polled:?}");
    assert!(!world.fake.subscribed(ACCOUNT));
    drop(account);
    world.fake.shutdown();
}
