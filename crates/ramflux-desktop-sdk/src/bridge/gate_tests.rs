// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The schema gate on the wire (S10c D1): deterministic held, failed and
//! timed-out status answers against a recording fake daemon. Every row
//! asserts the message-class requests the daemon actually received.
use super::*;
use crate::bus::BusRequest;
use crate::bus::approval::is_message_method;
use crate::config::{Deadlines, SchemaSupport};
use crate::receipts::Delivered;
use ramflux_desktop_core::{AccountId, SchemaGate};
use ramflux_desktop_test::fake_bus::{FakeBus, Reply, socket_path};
use serde_json::json;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

/// What the fake daemon answers to `daemon.status`.
#[derive(Clone)]
enum Status {
    Field(Value),
    Absent,
    Fail,
}

struct Daemon {
    status: Mutex<Status>,
    hold: AtomicBool,
    reset: AtomicBool,
    /// Holds `identity.directory` answers (a prerequisite of messages).
    identities: AtomicBool,
}

type Shared = Arc<Daemon>;

fn daemon(status: Status) -> Shared {
    Arc::new(Daemon {
        status: Mutex::new(status),
        hold: AtomicBool::new(false),
        reset: AtomicBool::new(false),
        identities: AtomicBool::new(false),
    })
}

fn start(name: &str, shared: &Shared) -> (FakeBus, std::path::PathBuf) {
    let path = socket_path(name);
    let shared = shared.clone();
    let fake = FakeBus::start(&path, move |method, _, _| {
        if method == "daemon.status" {
            while shared.hold.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            return match shared.status.lock().expect("status").clone() {
                Status::Field(value) => Reply::Ok(json!({ "schema_generation": value })),
                Status::Absent => Reply::Ok(json!({})),
                Status::Fail => Reply::Error("Unavailable", "status unavailable".to_owned()),
            };
        }
        if method == "identity.directory" {
            while shared.identities.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        if method == "account.status" && shared.reset.load(Ordering::SeqCst) {
            return Reply::Error("SchemaResetRequired", "older generation".to_owned());
        }
        Reply::Ok(json!({}))
    });
    (fake, path)
}

fn account_task(
    path: &std::path::Path,
    account: &str,
    support: SchemaSupport,
) -> (Task, Inbox, std_mpsc::Receiver<Delivery>) {
    account_task_with(path, account, support, Duration::from_secs(5))
}

fn account_task_with(
    path: &std::path::Path,
    account: &str,
    support: SchemaSupport,
    deadline: Duration,
) -> (Task, Inbox, std_mpsc::Receiver<Delivery>) {
    let mut config = SdkConfig::new(path);
    config.call_api = false;
    config.backup_api = false;
    config.settings_path = None;
    config.schema_support = support;
    config.reconnect_backoff = Duration::from_millis(60);
    config.deadlines = Deadlines::uniform(deadline);
    let (out, delivered) = std_mpsc::channel();
    let (phrases, _) = std_mpsc::channel();
    let (task, inbox) = build(
        config,
        Scope::Account(AccountId::new(account)),
        out,
        Ceremonies { phrases: phrases.into(), audit: Arc::default() },
    );
    (task, inbox, delivered)
}

/// Whether every read of the epoch has finished and nothing waits.
fn settled(task: &Task) -> bool {
    task.snapshot_task.is_none()
        && task.read_recovery.running.is_empty()
        && task.batch_pending == 0
        && task.held_release.is_none()
        && task.pending_markers == 0
}

async fn pump(task: &mut Task, inbox: &mut Inbox, mut done: impl FnMut(&Task) -> bool) {
    // A panic guard only: every wait below is on a causal condition.
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        while let Ok(status) = inbox.connection_states.try_recv() {
            task.connection_changed(status);
        }
        while let Ok(read) = inbox.internal.try_recv() {
            task.internal(read);
        }
        task.tick();
        if done(task) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the gate settles (epoch {}, {:?}, link {:?}, disconnected {})",
            task.gate.epoch(),
            task.gate.verdict(),
            task.link,
            task.transport_disconnected
        );
        tokio::task::yield_now().await;
        std::thread::sleep(Duration::from_micros(200));
    }
}

/// Runs until the epoch's reads have all finished.
async fn settle(task: &mut Task, inbox: &mut Inbox) {
    pump(task, inbox, settled).await;
}

/// Methods the daemon received that are message-class.
fn message_requests(fake: &FakeBus) -> Vec<String> {
    fake.seen()
        .into_iter()
        .map(|seen| seen.method)
        .filter(|method| {
            gate::is_message_method(method)
                || matches!(
                    method.as_str(),
                    "conversation.directory"
                        | "group.list"
                        | "object.directory"
                        | "call.list"
                        | "group.receive"
                )
        })
        .collect()
}

fn gates(delivered: &std_mpsc::Receiver<Delivery>) -> Vec<(SchemaGate, Option<i64>)> {
    delivered
        .try_iter()
        .filter_map(|d| match d.event {
            DomainEvent::SchemaGateChanged { gate, generation, .. } => Some((gate, generation)),
            _ => None,
        })
        .collect()
}

fn verdict(task: &Task) -> SchemaGate {
    task.gate.verdict()
}

fn seed_plaintext(task: &mut Task) {
    let events = task.translator.outgoing_message(
        &json!({ "context_id": "c-1", "text": "hello" }),
        "m-1",
        ramflux_desktop_core::Receipt::Sent,
        ramflux_desktop_core::Timestamp(1),
    );
    assert!(!events.is_empty());
    assert_eq!(task.translator.plaintext_len(), 1, "the oracle is seeded");
}

fn receipt() -> Delivered {
    Delivered {
        conversation_id: "c-1".to_owned(),
        message_id: "m-9".to_owned(),
        receiver_device_id: "dev-me".to_owned(),
        sender_device_id: "dev-ana".to_owned(),
        sender_target: "target-ana".to_owned(),
    }
}

/// Answers the gate's status as the daemon would on the current connection.
fn answer(task: &mut Task, generation: serde_json::Value) {
    let epoch = task.gate.epoch();
    let connection = task.client.connections_opened();
    task.internal(Internal::SchemaAnswer {
        epoch,
        generation: connection,
        result: Ok(json!({ "schema_generation": generation })),
    });
}

/// A dispatch/drain barrier for a held status: every non-message request of
/// the snapshot has reached the daemon (the tasks run in snapshot order on
/// this single-threaded runtime, so anything dispatched before the last of
/// them is on the wire too) and nothing but the `held` requests is still in
/// flight. A message-class read leaked ahead of the answer would be in the
/// daemon's log by now.
async fn status_held(task: &mut Task, inbox: &mut Inbox, fake: &FakeBus, held: usize) {
    const BATCH: [&str; 9] = [
        "daemon.status",
        "node.list",
        "device.list",
        "identity.directory",
        "contact.list",
        "grant.list",
        "mcp.approval.list",
        "mcp.tool.list",
        "mcp.server.list",
    ];
    pump(task, inbox, |t| {
        let seen = fake.seen();
        BATCH.iter().all(|method| seen.iter().any(|r| r.method == *method))
            && t.client.in_flight() == held
    })
    .await;
}

/// Answers where a held status makes `settle` impossible.
async fn status_asked(task: &mut Task, inbox: &mut Inbox, fake: &FakeBus) {
    pump(task, inbox, |_| fake.seen().iter().any(|r| r.method == "daemon.status")).await;
}

#[tokio::test]
async fn schema_gate_state_machine_blocks_message_traffic() {
    // Status held and the account snapshot first: its message reads wait in
    // the queue, in order; nothing message-class is on the wire until the
    // answer comes, and then the queue is sent.
    let shared = daemon(Status::Field(json!(0)));
    shared.hold.store(true, Ordering::SeqCst);
    let (fake, path) = start("gate-held", &shared);
    let (mut task, mut inbox, delivered) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    task.snapshot();
    status_held(&mut task, &mut inbox, &fake, 2).await;
    assert_eq!(verdict(&task), SchemaGate::Unresolved);
    assert_eq!(
        task.gate.queued_reads(),
        [
            Fetch::first(Paged::Groups),
            Fetch::first(Paged::Conversations),
            Fetch::first(Paged::Objects)
        ],
        "the snapshot's message reads are held in snapshot order"
    );
    assert_eq!(message_requests(&fake), Vec::<String>::new(), "zero message-class requests");
    assert!(fake.seen().iter().any(|r| r.method == "account.status"), "non-message reads flow");
    assert_eq!(gates(&delivered), [(SchemaGate::Unresolved, None)]);
    shared.hold.store(false, Ordering::SeqCst);
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    settle(&mut task, &mut inbox).await;
    let sent = message_requests(&fake);
    for method in ["group.list", "conversation.directory", "object.directory"] {
        assert!(sent.iter().any(|m| m == method), "{method} is sent after the release: {sent:?}");
    }
    assert_eq!(gates(&delivered), [(SchemaGate::Open, Some(0))]);
    fake.shutdown();

    // Status error: Failed, nothing sent; then a successful retry opens.
    let shared = daemon(Status::Fail);
    let (fake, path) = start("gate-error", &shared);
    let (mut task, mut inbox, delivered) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    task.snapshot();
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Failed).await;
    settle(&mut task, &mut inbox).await;
    assert_eq!(message_requests(&fake), Vec::<String>::new(), "failed: zero requests");
    assert!(gates(&delivered).iter().any(|(gate, _)| *gate == SchemaGate::Failed));
    *shared.status.lock().expect("status") = Status::Field(json!(0));
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    settle(&mut task, &mut inbox).await;
    assert!(!message_requests(&fake).is_empty(), "the retry opened the gate");
    fake.shutdown();

    // Status timeout: Failed as well.
    let shared = daemon(Status::Field(json!(0)));
    shared.hold.store(true, Ordering::SeqCst);
    let (fake, path) = start("gate-timeout", &shared);
    let (mut task, mut inbox, _delivered) =
        account_task_with(&path, "acct-main", SchemaSupport::Legacy, Duration::from_millis(250));
    task.snapshot();
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Failed).await;
    assert_eq!(message_requests(&fake), Vec::<String>::new(), "timed out: zero requests");
    shared.hold.store(false, Ordering::SeqCst);
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    fake.shutdown();
}

#[tokio::test]
async fn schema_gate_table_per_generation_and_support() {
    #[cfg(feature = "test-support")]
    let supports = [SchemaSupport::Legacy, SchemaSupport::V1Candidate];
    #[cfg(not(feature = "test-support"))]
    let supports = [SchemaSupport::Legacy];
    let rows: [(&str, Status); 6] = [
        ("absent", Status::Absent),
        ("0", Status::Field(json!(0))),
        ("1", Status::Field(json!(1))),
        ("2", Status::Field(json!(2))),
        ("-1", Status::Field(json!(-1))),
        ("\"1\"", Status::Field(json!("1"))),
    ];
    for support in supports {
        for (name, status) in rows.clone() {
            let want = gate::verdict(
                match &status {
                    Status::Absent => gate::Status::Absent,
                    Status::Field(v) => gate::classify(&json!({ "schema_generation": v })),
                    Status::Fail => gate::Status::Failed,
                },
                support,
            );
            let shared = daemon(status);
            let (fake, path) = start(&format!("gate-{name}-{support:?}"), &shared);
            let (mut task, mut inbox, _) = account_task(&path, "acct-main", support);
            task.snapshot();
            pump(&mut task, &mut inbox, |t| verdict(t) != SchemaGate::Unresolved).await;
            settle(&mut task, &mut inbox).await;
            assert_eq!(verdict(&task), want, "{name} under {support:?}");
            let sent = message_requests(&fake);
            assert_eq!(!sent.is_empty(), want.is_open(), "{name} under {support:?}: {sent:?}");
            fake.shutdown();
        }
    }
}

/// Every wire operation the plan builders can emit, and whether it is
/// message-class. A literal in `plan/*.rs` that is not here fails the
/// completeness test below.
const INVENTORY: &[(&str, bool)] = &[
    ("account.backup.cancel", false),
    ("account.backup.create", false),
    ("account.backup.export", false),
    ("account.backup.import", false),
    ("account.backup.jobs", false),
    ("account.create", false),
    ("account.export_agent_credentials", false),
    ("account.list", false),
    ("account.lock", false),
    ("account.passphrase.rotate", false),
    ("account.restore.install", false),
    ("account.restore.verify", false),
    ("account.status", false),
    ("account.unlock", false),
    ("call.answer", false),
    ("call.hangup", false),
    ("call.invite", false),
    ("call.list", true),
    ("call.media.session", false),
    ("call.reject", false),
    ("contact.accept", false),
    ("contact.add", false),
    ("contact.block", false),
    ("contact.card.export", false),
    ("contact.decline", false),
    ("contact.list", false),
    ("contact.reject", false),
    ("contact.remove", false),
    ("contact.requests.list", false),
    ("contact.requests.policy.set", false),
    ("contact.unblock", false),
    ("contact.verify", false),
    ("conversation.directory", true),
    ("conversation.disappearing.set", true),
    ("conversation.mute", true),
    ("conversation.start", true),
    ("daemon.status", false),
    ("device.list", false),
    ("device.revoke", false),
    ("grant.approve", false),
    ("grant.deny", false),
    ("grant.list", false),
    ("grant.list_standing_approvals", false),
    ("grant.revoke", false),
    ("grant.revoke_standing_approval", false),
    ("group.create", true),
    ("group.invite.create", true),
    ("group.list", true),
    ("group.member.add", true),
    ("group.member.ban", true),
    ("group.member.kick", true),
    ("group.member.remove", true),
    ("group.name.set", true),
    ("group.receive", true),
    ("group.role.set", true),
    ("group.send", true),
    ("identity.directory", false),
    ("mcp.approval.list", false),
    ("mcp.server.list", false),
    ("mcp.tool.list", false),
    ("message.list", true),
    ("message.receipt.delivered", true),
    ("message.submit", true),
    ("node.add", false),
    ("node.confirm", false),
    ("node.list", false),
    ("node.probe", false),
    ("node.remove", false),
    ("node.set_default", false),
    ("node.set_enabled", false),
    ("object.directory", true),
    ("object.get", true),
    ("object.transfer.resume", true),
    ("surface.action", true),
    ("surface.get", true),
    ("surface.list", true),
    ("surface.request_resync", true),
    ("task.accept", true),
    ("task.cancel", true),
    ("task.decline", true),
    ("task.get", true),
    ("task.list", true),
    ("task.resolve", true),
    ("task.start", true),
];

#[test]
fn every_planned_wire_operation_is_classified() {
    // Mechanical completeness: every method literal in the plan builders
    // (compound and derived requests included) is in the inventory, and the
    // classifier agrees with it.
    let sources = [
        include_str!("../plan/agents.rs"),
        include_str!("../plan/backup.rs"),
        include_str!("../plan/call.rs"),
        include_str!("../plan/contact.rs"),
        include_str!("../plan/contact_card.rs"),
        include_str!("../plan/conversation.rs"),
        include_str!("../plan/fetch.rs"),
        include_str!("../plan/group.rs"),
        include_str!("../plan/group_create.rs"),
        include_str!("../plan/identity.rs"),
        include_str!("../plan/message.rs"),
        include_str!("../plan/mod.rs"),
        include_str!("../plan/object.rs"),
        include_str!("../plan/surface.rs"),
        include_str!("../plan/task.rs"),
    ];
    let mut literals = std::collections::BTreeSet::new();
    for source in sources {
        for (index, part) in source.split('"').enumerate() {
            let method = index % 2 == 1
                && part.contains('.')
                && part.split('.').all(|word| {
                    !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase() || c == '_')
                })
                && [
                    "account",
                    "call",
                    "contact",
                    "conversation",
                    "daemon",
                    "device",
                    "grant",
                    "group",
                    "identity",
                    "mcp",
                    "message",
                    "node",
                    "object",
                    "surface",
                    "task",
                ]
                .contains(&part.split('.').next().unwrap_or_default());
            if method {
                literals.insert(part.to_owned());
            }
        }
    }
    for method in &literals {
        assert!(
            INVENTORY.iter().any(|(known, _)| known == method),
            "`{method}` is emitted by a plan builder but is not classified"
        );
    }
    for (method, message) in INVENTORY {
        assert_eq!(is_message_method(method), *message, "classification of {method}");
    }
}

#[tokio::test]
async fn every_message_class_wire_operation_is_refused_while_blocked_and_others_flow() {
    let shared = daemon(Status::Field(json!(2)));
    let (fake, path) = start("gate-inventory", &shared);
    let (mut task, mut inbox, _) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    task.snapshot();
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Unsupported).await;
    settle(&mut task, &mut inbox).await;
    let before = fake.seen().len();
    for (method, message) in INVENTORY {
        let request = BusRequest::new(Some("acct-main"), "x", *method, json!({}))
            .with_operation_id(format!("key-{method}"));
        let outcome = task.client.call(&request).await;
        let refused =
            matches!(&outcome, Err(BusError::Daemon { code, .. }) if code == "SchemaGateClosed");
        assert_eq!(
            refused, *message,
            "{method}: refused with SchemaGateClosed only if message-class"
        );
    }
    let sent: Vec<String> = fake.seen()[before..].iter().map(|r| r.method.clone()).collect();
    for (method, message) in INVENTORY {
        if *message {
            assert!(!sent.iter().any(|m| m == method), "{method} never reached the daemon");
        }
    }
    for method in ["account.list", "node.list", "contact.list", "device.list", "grant.list"] {
        assert!(sent.iter().any(|m| m == method), "{method} still flows");
    }
    // The user-facing path says the same: a send is refused at the effect.
    task.effect(Effect::Call {
        method: "conversation.start".to_owned(),
        body: json!({ "context_id": "c-9", "participant_id": "principal-ana", "text": "hi" }),
    });
    assert!(!message_requests(&fake).iter().any(|m| m == "message.submit"), "zero submissions");
    fake.shutdown();
}

#[tokio::test]
async fn a_reconnect_cannot_carry_a_message_ahead_of_the_new_epochs_status() {
    let shared = daemon(Status::Field(json!(0)));
    let (fake, path) = start("gate-restart", &shared);
    let (mut task, mut inbox, _) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    task.snapshot();
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    settle(&mut task, &mut inbox).await;
    let first_connection = task.client.connections_opened();
    fake.shutdown();

    // The daemon restarts with its status held; the scope has not noticed:
    // its approval is still the old connection's.
    let restarted = daemon(Status::Field(json!(0)));
    restarted.hold.store(true, Ordering::SeqCst);
    let (fake, _) = start("gate-restart", &restarted);
    assert_eq!(verdict(&task), SchemaGate::Open, "no notification was processed yet");
    let submit = BusRequest::new(Some("acct-main"), "message", "message.submit", json!({}))
        .with_operation_id("retry-key".to_owned());
    // The first attempt may find the old connection dead; the keyed retry
    // reconnects and must still be held back.
    let outcome = task.client.call(&submit).await;
    assert!(
        matches!(&outcome, Err(BusError::Daemon { code, .. }) if code == "SchemaGateClosed"),
        "the reconnected attempt is refused: {outcome:?}"
    );
    assert!(task.client.connections_opened() > first_connection, "it did reconnect");
    assert!(
        fake.seen().iter().all(|r| r.method != "message.submit"),
        "nothing message-class reached the restarted daemon"
    );
    // The reconnect is noticed through the connection notification alone
    // (no manual epoch): the same recovery asks the new connection for its
    // status, and only then does it carry messages.
    restarted.hold.store(false, Ordering::SeqCst);
    let known = task.gate_connection;
    pump(&mut task, &mut inbox, |t| {
        verdict(t) == SchemaGate::Open && t.gate_connection > known && t.pending_markers == 0
    })
    .await;
    settle(&mut task, &mut inbox).await;
    let outcome = task.client.call(&submit).await;
    assert!(
        !matches!(&outcome, Err(BusError::Daemon { code, .. }) if code == "SchemaGateClosed"),
        "approved on the new connection: {outcome:?}"
    );
    assert!(fake.seen().iter().any(|r| r.method == "message.submit"), "messages flow again");
    fake.shutdown();
}

#[tokio::test]
async fn reconnect_revokes_the_approval_and_a_late_old_epoch_answer_cannot_reopen() {
    let shared = daemon(Status::Field(json!(0)));
    let (fake, path) = start("gate-reconnect", &shared);
    let (mut task, mut inbox, delivered) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    task.snapshot();
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    settle(&mut task, &mut inbox).await;
    assert!(!message_requests(&fake).is_empty());
    let first_epoch = task.gate.epoch();
    seed_plaintext(&mut task);
    // A retry that is not due yet when the connection drops.
    task.receipts
        .as_mut()
        .expect("receipts")
        .observe(vec![receipt()], Instant::now() + Duration::from_secs(30));
    assert_eq!(task.receipts.as_ref().map(Receipts::pending_len), Some(1));
    delivered.try_iter().for_each(drop);
    fake.shutdown();

    // The daemon restarts with its status held.
    let restarted = daemon(Status::Field(json!(0)));
    restarted.hold.store(true, Ordering::SeqCst);
    let (fake, _) = start("gate-reconnect", &restarted);
    // A request on the dead connection finds it lost; the scope re-attaches
    // after its backoff, in a new epoch.
    task.spawn_fetches(vec![Fetch::AccountStatus(AccountId::new("acct-main"))]);
    pump(&mut task, &mut inbox, |t| t.gate.epoch() > first_epoch).await;
    status_asked(&mut task, &mut inbox, &fake).await;
    assert_eq!(verdict(&task), SchemaGate::Unresolved, "the approval is revoked");
    assert_eq!(message_requests(&fake), Vec::<String>::new(), "zero message-class requests");
    assert_eq!(task.translator.plaintext_len(), 0, "caches are empty at the data layer");
    assert_eq!(
        task.receipts.as_ref().map(Receipts::pending_len),
        Some(0),
        "receipt retries dropped"
    );
    assert!(
        fake.seen().iter().all(|r| r.method != "message.receipt.delivered"),
        "no receipt is sent while unresolved"
    );
    // An old epoch's compatible answer cannot open the gate.
    task.internal(Internal::SchemaAnswer {
        epoch: first_epoch,
        generation: task.client.connections_opened(),
        result: Ok(json!({ "schema_generation": 0 })),
    });
    assert_eq!(verdict(&task), SchemaGate::Unresolved, "a late old-epoch answer is ignored");
    restarted.hold.store(false, Ordering::SeqCst);
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    fake.shutdown();
}

#[tokio::test]
async fn an_older_snapshot_arriving_after_the_new_epoch_cannot_reopen_the_gate() {
    let shared = daemon(Status::Field(json!(0)));
    let (fake, path) = start("gate-old-snapshot", &shared);
    let (mut task, mut inbox, delivered) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    task.snapshot();
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    settle(&mut task, &mut inbox).await;
    let old_epoch = task.gate.epoch();
    // The daemon now reports an incompatible generation; a new epoch asks.
    *shared.status.lock().expect("status") = Status::Field(json!(2));
    task.snapshot();
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Unsupported).await;
    settle(&mut task, &mut inbox).await;
    // The old epoch's snapshot (status 0, and a capability it alone
    // reports) completes only now: nothing of it is translated.
    delivered.try_iter().for_each(drop);
    task.internal(Internal::Snapshot(
        old_epoch,
        vec![(
            Fetch::DaemonStatus,
            None,
            Ok(json!({ "schema_generation": 0, "born_v3_creation": true })),
        )],
    ));
    assert_eq!(verdict(&task), SchemaGate::Unsupported, "the older answer decides nothing");
    assert!(
        !delivered.try_iter().any(|d| matches!(d.event, DomainEvent::CapabilitiesUpdated(_))),
        "an older epoch's snapshot is not translated"
    );
    // The same holds for a single read queued before the new epoch began.
    task.pending_markers = 1;
    task.internal(Internal::Fetched((
        Fetch::DaemonStatus,
        None,
        Ok(json!({ "schema_generation": 0 })),
    )));
    task.pending_markers = 0;
    assert_eq!(verdict(&task), SchemaGate::Unsupported);
    fake.shutdown();
}

#[tokio::test]
async fn a_changed_status_in_the_same_epoch_blocks_at_once_and_late_results_are_rejected() {
    let shared = daemon(Status::Field(json!(0)));
    let (fake, path) = start("gate-change", &shared);
    let (mut task, mut inbox, delivered) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    task.snapshot();
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    settle(&mut task, &mut inbox).await;
    seed_plaintext(&mut task);
    delivered.try_iter().for_each(drop);
    let before = fake.seen().len();
    // The next status read says generation 1: this build cannot open it.
    task.on_read(&Fetch::DaemonStatus, Ok(json!({ "schema_generation": 1 })));
    assert_eq!(verdict(&task), SchemaGate::UpdateApp);
    assert_eq!(task.translator.plaintext_len(), 0, "translator caches are dropped");
    assert_eq!(gates(&delivered), [(SchemaGate::UpdateApp, Some(1))]);
    // A group receive that was in flight completes late: not translated.
    let late = Fetch::GroupReceive {
        context: ramflux_desktop_core::ContextId::new("c-1"),
        group: "g-1".to_owned(),
        title: None,
    };
    task.internal(Internal::Fetched((
        late.clone(),
        None,
        Ok(json!({ "decrypted_messages": [{ "message_id": "m-late", "body_utf8": "late" }] })),
    )));
    assert_eq!(task.translator.plaintext_len(), 0, "a late result does not repopulate");
    // Message-class effects are refused; nothing reaches the daemon.
    task.effect(Effect::Call {
        method: "message.send".to_owned(),
        body: json!({ "context_id": "c-1", "text": "hi" }),
    });
    let failed = delivered.try_iter().any(|d| {
        matches!(d.event, DomainEvent::EffectFailed(f)
            if matches!(&f.code, ramflux_desktop_core::FailureCode::Daemon(c) if c == "SchemaGateClosed"))
    });
    assert!(failed, "the send is refused with the gate's code");
    task.spawn_fetches(vec![Fetch::Messages(ramflux_desktop_core::ContextId::new("c-1"))]);
    settle(&mut task, &mut inbox).await;
    assert_eq!(
        fake.seen()[before..].iter().filter(|r| r.method == "message.list").count(),
        0,
        "blocked reads are dropped"
    );
    // Back to a readable generation, checked against its connection: the
    // dropped work is read again, and only the fresh read fills the cache.
    answer(&mut task, json!(0));
    assert_eq!(verdict(&task), SchemaGate::Open);
    settle(&mut task, &mut inbox).await;
    assert!(
        fake.seen()[before..].iter().any(|r| r.method == "conversation.directory"),
        "the messages are read again after opening"
    );
    assert_eq!(task.translator.plaintext_len(), 0, "nothing came from the late result");
    fake.shutdown();
}

#[tokio::test]
async fn messages_wait_for_the_identities_and_devices_they_are_attributed_by() {
    let shared = daemon(Status::Field(json!(0)));
    shared.identities.store(true, Ordering::SeqCst);
    let (fake, path) = start("gate-order", &shared);
    let (mut task, mut inbox, _) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    task.snapshot();
    // The status is in and the gate open, but the identity read is held.
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open && t.held_release.is_some())
        .await;
    assert_eq!(message_requests(&fake), Vec::<String>::new(), "no message read before identities");
    shared.identities.store(false, Ordering::SeqCst);
    settle(&mut task, &mut inbox).await;
    let order: Vec<String> = fake.seen().iter().map(|r| r.method.clone()).collect();
    let identities = order.iter().position(|m| m == "identity.directory").expect("identities");
    let messages = order.iter().position(|m| m == "conversation.directory").expect("messages");
    assert!(identities < messages, "identities were read first: {order:?}");
    fake.shutdown();
}

#[tokio::test]
async fn each_account_scope_holds_its_own_queue_while_unresolved() {
    let shared = daemon(Status::Field(json!(0)));
    shared.hold.store(true, Ordering::SeqCst);
    let (fake, path) = start("gate-accounts", &shared);
    let (mut first, mut first_inbox, _) = account_task(&path, "acct-one", SchemaSupport::Legacy);
    let (mut second, mut second_inbox, _) = account_task(&path, "acct-two", SchemaSupport::Legacy);
    first.snapshot();
    second.snapshot();
    settle(&mut first, &mut first_inbox).await;
    settle(&mut second, &mut second_inbox).await;
    assert_eq!(first.gate.queued(), 3);
    assert_eq!(second.gate.queued(), 3, "per-account queues");
    assert_eq!(message_requests(&fake), Vec::<String>::new(), "nothing is sent");
    fake.shutdown();
}

#[tokio::test]
async fn an_account_from_an_older_build_is_never_opened_or_written() {
    let shared = daemon(Status::Field(json!(0)));
    shared.reset.store(true, Ordering::SeqCst);
    let (fake, path) = start("gate-reset", &shared);
    let (mut task, mut inbox, delivered) = account_task(&path, "acct-old", SchemaSupport::Legacy);
    task.snapshot();
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::ResetRequired).await;
    settle(&mut task, &mut inbox).await;
    assert!(gates(&delivered).iter().any(|(gate, _)| *gate == SchemaGate::ResetRequired));
    let after = message_requests(&fake).len();
    task.spawn_fetches(vec![Fetch::Messages(ramflux_desktop_core::ContextId::new("c-1"))]);
    settle(&mut task, &mut inbox).await;
    assert_eq!(
        message_requests(&fake).len(),
        after,
        "nothing message-class is sent once it is known"
    );
    // Only reads were made: nothing that writes, deletes or converts.
    for seen in fake.seen() {
        let method = seen.method.as_str();
        assert!(
            !method.contains("delete")
                && !method.contains("remove")
                && !method.contains("create")
                && !method.contains("convert")
                && !method.contains("migrate")
                && !method.contains("restore"),
            "no destructive request: {method}"
        );
    }
    fake.shutdown();
}

#[tokio::test]
async fn an_older_epochs_single_read_completing_after_reopening_is_not_translated() {
    let shared = daemon(Status::Field(json!(0)));
    let (fake, path) = start("gate-old-read", &shared);
    let (mut task, mut inbox, _) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    task.snapshot();
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    settle(&mut task, &mut inbox).await;
    let old_epoch = task.gate.epoch();
    // A group receive is outstanding when the connection changes: a new
    // epoch begins, closes on an incompatible status, and reopens.
    *shared.status.lock().expect("status") = Status::Field(json!(2));
    task.snapshot();
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Unsupported).await;
    settle(&mut task, &mut inbox).await;
    *shared.status.lock().expect("status") = Status::Field(json!(0));
    task.recheck_schema();
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    settle(&mut task, &mut inbox).await;
    assert_eq!(task.pending_markers, 0, "the epoch marker has been seen");
    assert_eq!(task.translator.plaintext_len(), 0);
    // The old epoch's receive completes only now.
    let late = Fetch::GroupReceive {
        context: ramflux_desktop_core::ContextId::new("c-1"),
        group: "g-1".to_owned(),
        title: None,
    };
    task.internal(Internal::FetchedAt {
        epoch: old_epoch,
        read: (
            late,
            None,
            Ok(json!({ "decrypted_messages": [{ "message_id": "m-old", "body_utf8": "old" }] })),
        ),
    });
    assert_eq!(
        task.translator.plaintext_len(),
        0,
        "an older epoch's result is dropped untranslated"
    );
    fake.shutdown();
}

#[tokio::test]
async fn recovery_of_a_changed_connection_holds_messages_behind_the_identities() {
    let shared = daemon(Status::Field(json!(0)));
    let (fake, path) = start("gate-recover-order", &shared);
    let (mut task, mut inbox, _) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    task.snapshot();
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    settle(&mut task, &mut inbox).await;
    let known = task.gate_connection;
    fake.shutdown();

    // The daemon restarts with the identity directory held. A keyed resend
    // reconnects; the connection notification starts the recovery.
    let restarted = daemon(Status::Field(json!(0)));
    restarted.identities.store(true, Ordering::SeqCst);
    let (fake, _) = start("gate-recover-order", &restarted);
    let submit = BusRequest::new(Some("acct-main"), "message", "message.submit", json!({}))
        .with_operation_id("recover-key".to_owned());
    let _ = task.client.call(&submit).await;
    // The status resolves open, but the identities have not arrived: the
    // messages stay behind them.
    pump(&mut task, &mut inbox, |t| {
        verdict(t) == SchemaGate::Open && t.gate_connection > known && t.held_release.is_some()
    })
    .await;
    assert_eq!(
        message_requests(&fake).iter().filter(|m| *m != "message.submit").count(),
        0,
        "no message read is released while the identities are held"
    );
    restarted.identities.store(false, Ordering::SeqCst);
    settle(&mut task, &mut inbox).await;
    let order: Vec<String> = fake.seen().iter().map(|r| r.method.clone()).collect();
    let identities = order.iter().position(|m| m == "identity.directory").expect("identities");
    let messages = order.iter().position(|m| m == "conversation.directory").expect("messages");
    assert!(identities < messages, "identities were read first: {order:?}");
    fake.shutdown();
}
