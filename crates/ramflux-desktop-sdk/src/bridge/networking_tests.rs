// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Networking recovery through the production bridge and field adapter.
use super::*;
use crate::bus::stream::{StreamEnd, Streamed};
use ramflux_desktop_core::{AccountId, DomainEvent, Networking};
use ramflux_desktop_test::fake_bus::{FakeBus, Reply, socket_path};
use serde_json::json;

fn task() -> (Task, Inbox, std_mpsc::Receiver<Delivery>) {
    let mut config = SdkConfig::new("network-test.sock");
    config.settings_path = None;
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
    task.open_gate_for_tests();
    (task, inbox, delivered)
}

fn pending_status() -> Value {
    json!({"state":"unlocked", "networking":"live", "networking_snapshot":{
        "state":"networking_pending", "phase":"backoff", "generation":3,
        "cause":{"kind":"node_unreachable", "retry_after_ms":5000}, "retry_after_ms":5000
    }})
}

fn latest(delivered: &std_mpsc::Receiver<Delivery>) -> ramflux_desktop_core::Account {
    delivered
        .try_iter()
        .filter_map(|d| match d.event {
            DomainEvent::AccountUpdated(a) => Some(a),
            _ => None,
        })
        .last()
        .expect("account update")
}

async fn answer(task: &mut Task, inbox: &mut Inbox) {
    task.internal(
        tokio::time::timeout(Duration::from_secs(3), inbox.internal.recv())
            .await
            .expect("deadline")
            .expect("answer"),
    );
}

#[tokio::test]
async fn admitted_disconnected_account_attaches_as_pending() {
    let path = socket_path("network-attach");
    let fake = FakeBus::start(&path, |_, _, _| Reply::Ok(pending_status()));
    let (mut task, mut inbox, delivered) = task();
    task.client = BusClient::new(path, Duration::from_secs(2), task.config.deadlines);
    task.link = Link::Attaching;
    task.spawn_fetches(vec![Fetch::AccountStatus(AccountId::new("acct-main"))]);
    answer(&mut task, &mut inbox).await;
    let account = latest(&delivered);
    assert_eq!(account.lock_state, ramflux_desktop_core::LockState::Unlocked);
    assert_eq!(account.networking, Networking::Pending);
    assert_eq!(account.networking_reason.expect("cause").retry_after_ms, Some(5000));
    assert_eq!(fake.seen().len(), 1);
}

#[tokio::test]
async fn expired_cursor_recovers_networking_snapshot() {
    let path = socket_path("network-gap");
    let fake = FakeBus::start(&path, |request, _, _| {
        Reply::Ok(if request == "account.status" { pending_status() } else { json!({}) })
    });
    let (mut task, mut inbox, delivered) = task();
    task.client = BusClient::new(path, Duration::from_secs(2), task.config.deadlines);
    task.sync.cursor = Some(7);
    task.on_streamed(Streamed::Ended(StreamEnd::ResyncRequired));
    tokio::task::yield_now().await;
    assert!(fake.seen().is_empty(), "no reload before subscription opens");
    task.on_streamed(Streamed::Opened { head_seq: 20, floor_seq: 10 });
    assert_eq!(task.sync.cursor, Some(20));
    for _ in 0..20 {
        answer(&mut task, &mut inbox).await;
        if fake.seen().iter().any(|r| r.method == "account.status")
            && !task
                .read_recovery
                .running
                .contains_key(&Fetch::AccountStatus(AccountId::new("acct-main")))
        {
            break;
        }
    }
    assert_eq!(latest(&delivered).networking, Networking::Pending);
}

#[tokio::test]
async fn delayed_status_cannot_overwrite_newer_networking_event() {
    let path = socket_path("network-fence");
    let _fake = FakeBus::start(&path, |_, _, _| {
        Reply::Slow(
            Duration::from_millis(50),
            json!({"node_id":"node-local", "networking":"live", "networking_snapshot":{"state":"live"}}),
        )
    });
    let (mut task, mut inbox, delivered) = task();
    task.client = BusClient::new(path, Duration::from_secs(2), task.config.deadlines);
    task.spawn_fetches(vec![Fetch::AccountStatus(AccountId::new("acct-main"))]);
    let event = json!({"account_id":"acct-main", "state":"live", "session":pending_status()["networking_snapshot"]});
    task.networking_event(&event);
    assert_eq!(latest(&delivered).networking, Networking::Pending);
    answer(&mut task, &mut inbox).await;
    let local = latest(&delivered);
    assert_eq!(local.networking, Networking::Pending, "late snapshot must not restore false Live");
    assert_eq!(
        local.lock_state,
        ramflux_desktop_core::LockState::Unlocked,
        "local readiness survives the networking fence"
    );
    assert_eq!(local.node_id.as_ref().map(|id| id.as_str()), Some("node-local"));
    // Both account-list custody and lifecycle answers preserve the resident half.
    let translated = task.translator.fetched(
        &Fetch::AccountList,
        &json!({"accounts":["acct-main"],
        "account_details":[{"id":"acct-main", "state":"unlocked", "networking":"live"}]}),
        now(),
    );
    task.emit(translated.events);
    task.on_read(
        &Fetch::AccountStatus(AccountId::new("acct-main")),
        Ok(json!({"networking":"live"})),
    );
    let lifecycle = task.translator.effect_done(
        "account.unlock",
        &json!({"local_account_id":"acct-main"}),
        &json!({"local_account_id":"acct-main", "state":"unlocked", "networking":"live"}),
        now(),
    );
    task.emit(lifecycle);
    assert!(!delivered.try_iter().any(
        |d| matches!(d.event, DomainEvent::AccountUpdated(a) if a.networking == Networking::Live)
    ));
}

// P0's additive event body: custody stays live while the resident session changes.
fn event(state: &str) -> Value {
    json!({"account_id":"acct-main", "state":"live", "node_binding":null,
        "networking_reason":null, "session":{"state":state,"phase":if state == "live" {"live"} else {"backoff"},
        "generation":2,"cause":if state == "live" {Value::Null} else {json!({"kind":"node_unreachable","retry_after_ms":5000})},"retry_after_ms":5000}})
}

#[tokio::test]
async fn networking_changed_event_updates_header_without_status_read() {
    let path = socket_path("network-event");
    let fake = FakeBus::start(&path, |_, _, _| Reply::Ok(json!({})));
    let (mut task, _, delivered) = task();
    task.client = BusClient::new(path, Duration::from_secs(2), task.config.deadlines);
    task.on_streamed(Streamed::Event {
        seq: Some(1),
        at: None,
        topic: "account.networking.changed".into(),
        body: event("networking_pending"),
    });
    let account = latest(&delivered);
    assert_eq!(account.networking, Networking::Pending);
    assert_eq!(
        account.networking_reason.expect("session cause").kind,
        ramflux_desktop_core::NetworkingReasonKind::NodeUnreachable
    );
    task.on_streamed(Streamed::Event {
        seq: Some(2),
        at: None,
        topic: "account.networking.changed".into(),
        body: event("live"),
    });
    assert_eq!(latest(&delivered).networking, Networking::Live);
    assert!(fake.seen().is_empty(), "events never trigger status reads");
    task.on_pushed(Pushed::Event {
        topic: "account.networking.changed".into(),
        body: event("networking_pending"),
    });
    assert_eq!(latest(&delivered).networking, Networking::Pending);
    assert!(fake.seen().is_empty());
}

#[tokio::test]
async fn missed_networking_event_is_replayed_from_cursor() {
    let path = socket_path("network-replay");
    let fake = FakeBus::start(&path, |_, _, _| Reply::Ok(json!({})));
    fake.enable_stream();
    fake.publish("acct-main", "account.networking.changed", event("live"));
    fake.publish("acct-main", "account.networking.changed", event("networking_pending"));
    let (mut task, mut inbox, delivered) = task();
    task.config.socket_path = path;
    task.sync.cursor = Some(1);
    task.open_stream();
    for _ in 0..2 {
        let frame = tokio::time::timeout(Duration::from_secs(3), inbox.streamed.recv())
            .await
            .expect("deadline")
            .expect("stream frame");
        task.on_streamed(frame);
    }
    assert_eq!(latest(&delivered).networking, Networking::Pending);
    assert_eq!(task.sync.cursor, Some(2));
    let seen = fake.seen();
    let subscription = seen.iter().find(|r| r.method == "bus.subscribe").expect("subscription");
    assert_eq!(subscription.body["since_seq"], 1);
    assert!(
        subscription.body["topics"]
            .as_array()
            .expect("topics")
            .contains(&json!("account.networking.changed"))
    );
    assert!(!seen.iter().any(|r| r.method == "account.status"));
    task.on_streamed(Streamed::Event {
        seq: Some(2),
        at: None,
        topic: "account.networking.changed".into(),
        body: event("live"),
    });
    assert!(
        !delivered.try_iter().any(|d| matches!(d.event, DomainEvent::AccountUpdated(_))),
        "duplicate is ignored"
    );
}

#[tokio::test]
async fn custody_pending_and_authority_blocked_override_live_session() {
    let (mut task, _, delivered) = task();
    for blocked in [false, true] {
        task.on_read(
            &Fetch::AccountStatus(AccountId::new("acct-main")),
            Ok(json!({"networking":"networking_pending", "authority_blocked":blocked})),
        );
        task.networking_event(&json!({"account_id":"acct-main", "state":"networking_pending",
            "networking_reason":null, "session":{"state":"live"}}));
        let account = latest(&delivered);
        assert_eq!(account.networking_session, Some(Networking::Live));
        assert_eq!(account.networking_custody, Some(Networking::Pending));
        assert_eq!(account.networking, Networking::Pending);
        assert_eq!(account.authority_blocked, blocked);
    }
}

fn drain_store(
    delivered: &std_mpsc::Receiver<Delivery>,
    store: &mut ramflux_desktop_core::Store,
    seq: &mut u64,
) {
    for d in delivered.try_iter() {
        *seq += 1;
        store.apply(&ramflux_desktop_core::EventEnvelope::new(*seq, d.at, d.event)).expect("apply");
    }
}

#[tokio::test]
async fn daemon_list_and_lifecycle_merge_preserves_account_session() {
    for state in ["live", "networking_pending"] {
        let (mut task, _, delivered) = task();
        task.networking_event(&event(state));
        let mut store = ramflux_desktop_core::Store::new();
        let mut seq = 0;
        drain_store(&delivered, &mut store, &mut seq);
        let mut daemon = crate::translate::Translator::new(Scope::Daemon, "test.sock");
        let updates = daemon.fetched(&Fetch::AccountList,
            &json!({"accounts":["acct-main"], "account_details":[{"id":"acct-main", "state":"unlocked", "networking":"live"}]}), now());
        task.emit(updates.events);
        drain_store(&delivered, &mut store, &mut seq);
        let expected = if state == "live" { Networking::Live } else { Networking::Pending };
        let id = AccountId::new("acct-main");
        assert_eq!(store.account(&id).expect("account").networking_session, Some(expected));
        assert_eq!(store.account(&id).expect("account").networking, expected);
        task.emit(daemon.effect_done(
            "account.unlock",
            &json!({"local_account_id":"acct-main"}),
            &json!({"local_account_id":"acct-main", "state":"unlocked", "networking":"live"}),
            now(),
        ));
        drain_store(&delivered, &mut store, &mut seq);
        assert_eq!(store.account(&id).expect("account").networking_session, Some(expected));
        assert_eq!(store.account(&id).expect("account").networking, expected);
    }
}

#[tokio::test]
async fn push_gap_reopen_recovers_networking_snapshot() {
    let path = socket_path("network-push-gap");
    let fake = FakeBus::start(&path, |_, _, _| Reply::Ok(pending_status()));
    let (mut task, mut inbox, delivered) = task();
    task.client = BusClient::new(path, Duration::from_secs(2), task.config.deadlines);
    task.networking_event(&event("live"));
    assert_eq!(latest(&delivered).networking, Networking::Live);
    task.on_pushed(Pushed::Lost(BusError::Timeout));
    assert!(fake.seen().is_empty());
    task.on_pushed(Pushed::Opened);
    answer(&mut task, &mut inbox).await;
    assert_eq!(latest(&delivered).networking, Networking::Pending);
    assert_eq!(fake.seen().iter().filter(|r| r.method == "account.status").count(), 1);
}

#[tokio::test]
async fn sessionless_terminal_reason_survives_delayed_status() {
    let path = socket_path("network-sessionless-fence");
    let _fake = FakeBus::start(&path, |_, _, _| {
        Reply::Slow(Duration::from_millis(50), json!({"networking":"live"}))
    });
    let (mut task, mut inbox, delivered) = task();
    task.client = BusClient::new(path, Duration::from_secs(2), task.config.deadlines);
    task.spawn_fetches(vec![Fetch::AccountStatus(AccountId::new("acct-main"))]);
    task.networking_event(&json!({"account_id":"acct-main", "state":"networking_pending", "networking_reason":{"kind":"custody_expired"}}));
    latest(&delivered);
    answer(&mut task, &mut inbox).await;
    let account = latest(&delivered);
    assert_eq!(account.networking_session, None);
    assert_eq!(
        account.networking_reason.expect("terminal reason").kind,
        ramflux_desktop_core::NetworkingReasonKind::CustodyExpired
    );
}

#[tokio::test]
async fn authority_restored_event_clears_blocked_and_fences_delayed_status() {
    let (mut task, _, delivered) = task();
    let fetch = Fetch::AccountStatus(AccountId::new("acct-main"));
    let blocked = json!({"state":"unlocked", "networking":"networking_pending",
        "authority_blocked":true, "networking_snapshot":{"state":"live"}});
    task.on_read(&fetch, Ok(blocked.clone()));
    assert!(latest(&delivered).authority_blocked);
    let old_revision = task.networking_revision;
    // A Live resident session alone does not establish custody authority.
    task.networking_event(&json!({"account_id":"acct-main", "session":{"state":"live"}}));
    assert!(!delivered.try_iter().any(|d| matches!(d.event,
        DomainEvent::AccountUpdated(a) if !a.authority_blocked || a.networking == Networking::Live)));
    task.networking_event(&event("live"));
    let restored = latest(&delivered);
    assert!(!restored.authority_blocked);
    assert_eq!(restored.networking, Networking::Live);
    // A status dispatched before the event cannot reintroduce the old block.
    let mut delayed = blocked;
    delayed["node_id"] = json!("node-local");
    task.internal(Internal::Fetched((fetch, Some(old_revision), Ok(delayed))));
    let fenced = latest(&delivered);
    assert!(!fenced.authority_blocked);
    assert_eq!(fenced.networking, Networking::Live);
    assert_eq!(fenced.node_id.as_ref().map(|id| id.as_str()), Some("node-local"));
    // The translator itself must retain the cleared flag on later events.
    task.networking_event(&event("networking_pending"));
    assert!(!latest(&delivered).authority_blocked);
}
