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
use std::sync::{Condvar, MutexGuard};

/// A request the fake daemon holds until the test releases it. The handler
/// thread sleeps on a condition variable (no polling), is woken the moment
/// the latch opens, and gives up after a panic guard so a test that fails
/// before releasing cannot leave the thread running.
struct Latch {
    closed: Mutex<bool>,
    changed: Condvar,
    /// Set when a waiter gave up at the guard: the test fails at teardown.
    timed_out: AtomicBool,
}

impl Latch {
    /// Longer than any wait a row makes; a guard only.
    const GUARD: Duration = Duration::from_secs(30);

    fn new() -> Self {
        Self {
            closed: Mutex::new(false),
            changed: Condvar::new(),
            timed_out: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> MutexGuard<'_, bool> {
        self.closed.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn is_closed(&self) -> bool {
        *self.lock()
    }

    fn close(&self) {
        *self.lock() = true;
    }

    fn release(&self) {
        *self.lock() = false;
        self.changed.notify_all();
    }

    /// Waits until the latch opens. The guard is a failure, never a release:
    /// the handler panics and the test fails at teardown.
    fn wait(&self) {
        let closed = self.lock();
        let (closed, result) = self
            .changed
            .wait_timeout_while(closed, Self::GUARD, |closed| *closed)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if result.timed_out() && *closed {
            self.timed_out.store(true, Ordering::SeqCst);
            drop(closed);
            panic!("a held request was never released within the guard");
        }
    }
}

/// Releases every latch of the daemon when the test ends, however it ends.
struct Teardown(Shared);

impl Drop for Teardown {
    fn drop(&mut self) {
        self.0.hold.release();
        self.0.identities.release();
        self.0.first_status.release();
        let failed = [&self.0.hold, &self.0.identities, &self.0.first_status]
            .iter()
            .any(|latch| latch.timed_out.load(Ordering::SeqCst));
        if failed && !std::thread::panicking() {
            panic!("a fake-daemon handler gave up waiting on a latch");
        }
    }
}

/// What the fake daemon answers to `daemon.status`.
#[derive(Clone)]
enum Status {
    Field(Value),
    Absent,
    Fail,
}

struct Daemon {
    status: Mutex<Status>,
    hold: Latch,
    reset: AtomicBool,
    /// Holds `identity.directory` answers (a prerequisite of messages).
    identities: Latch,
    /// Holds only the first `daemon.status` request (the gate's own), until
    /// cleared; the snapshot's status read is answered.
    first_status: Latch,
    first_seen: AtomicBool,
}

type Shared = Arc<Daemon>;

fn daemon(status: Status) -> Shared {
    Arc::new(Daemon {
        status: Mutex::new(status),
        hold: Latch::new(),
        reset: AtomicBool::new(false),
        identities: Latch::new(),
        first_status: Latch::new(),
        first_seen: AtomicBool::new(false),
    })
}

fn start(name: &str, shared: &Shared) -> (FakeBus, std::path::PathBuf) {
    let path = socket_path(name);
    let shared = shared.clone();
    let fake = FakeBus::start(&path, move |method, _, _| {
        if method == "daemon.status" {
            if shared.first_status.is_closed() && !shared.first_seen.swap(true, Ordering::SeqCst) {
                shared.first_status.wait();
            }
            shared.hold.wait();
            return match shared.status.lock().expect("status").clone() {
                Status::Field(value) => Reply::Ok(json!({ "schema_generation": value })),
                Status::Absent => Reply::Ok(json!({})),
                Status::Fail => Reply::Error("Unavailable", "status unavailable".to_owned()),
            };
        }
        if method == "identity.directory" {
            shared.identities.wait();
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
        && !task.gate.prerequisites_pending()
        && task.pending_markers == 0
}

/// Attaches the scope: a live connection is recovered at once, a dead one is
/// opened first, so wait until the epoch has begun.
async fn attach(task: &mut Task, inbox: &mut Inbox) {
    let before = task.gate.epoch();
    task.snapshot();
    if task.gate.epoch() == before {
        pump(task, inbox, |t| t.gate.epoch() > before).await;
    }
}

async fn pump(task: &mut Task, inbox: &mut Inbox, done: impl FnMut(&Task) -> bool) {
    pump_with(task, inbox, true, done).await;
}

/// `pump`, optionally leaving the connection notifications queued so a row
/// can deliver them in an order of its own.
async fn pump_with(
    task: &mut Task,
    inbox: &mut Inbox,
    notifications: bool,
    mut done: impl FnMut(&Task) -> bool,
) {
    // A panic guard only: every wait below is on a causal condition.
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        while notifications && let Ok(status) = inbox.connection_states.try_recv() {
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
        stamp: Stamp { epoch, attempt: task.read_attempt },
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
    let _teardown_shared = Teardown(shared.clone());
    shared.hold.close();
    let (fake, path) = start("gate-held", &shared);
    let (mut task, mut inbox, delivered) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    attach(&mut task, &mut inbox).await;
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
    shared.hold.release();
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
    let _teardown_shared = Teardown(shared.clone());
    let (fake, path) = start("gate-error", &shared);
    let (mut task, mut inbox, delivered) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    attach(&mut task, &mut inbox).await;
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
    let _teardown_shared = Teardown(shared.clone());
    shared.hold.close();
    let (fake, path) = start("gate-timeout", &shared);
    let (mut task, mut inbox, _delivered) =
        account_task_with(&path, "acct-main", SchemaSupport::Legacy, Duration::from_millis(250));
    attach(&mut task, &mut inbox).await;
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Failed).await;
    assert_eq!(message_requests(&fake), Vec::<String>::new(), "timed out: zero requests");
    shared.hold.release();
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
            attach(&mut task, &mut inbox).await;
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
    attach(&mut task, &mut inbox).await;
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
    let _teardown_shared = Teardown(shared.clone());
    let (fake, path) = start("gate-restart", &shared);
    let (mut task, mut inbox, _) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    attach(&mut task, &mut inbox).await;
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    settle(&mut task, &mut inbox).await;
    let first_connection = task.client.connections_opened();
    fake.shutdown();

    // The daemon restarts with its status held; the scope has not noticed:
    // its approval is still the old connection's.
    let restarted = daemon(Status::Field(json!(0)));
    let _teardown_restarted = Teardown(restarted.clone());
    restarted.hold.close();
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
    restarted.hold.release();
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

/// The next connection notification, which must be `(generation, connected)`.
async fn next_state(
    inbox: &mut Inbox,
    generation: u64,
    connected: bool,
) -> crate::bus::client::ConnectionState {
    let state = tokio::time::timeout(Duration::from_secs(20), inbox.connection_states.recv())
        .await
        .expect("a connection notification arrives")
        .expect("the notification channel is open");
    assert_eq!((state.generation, state.connected), (generation, connected));
    state
}

#[tokio::test]
async fn a_second_reconnect_after_a_settled_one_recovers_in_its_own_epoch() {
    let a = daemon(Status::Field(json!(0)));
    let _teardown_a = Teardown(a.clone());
    let (fake, path) = start("gate-two-reconnects", &a);
    let (mut task, mut inbox, _) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    attach(&mut task, &mut inbox).await;
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    settle(&mut task, &mut inbox).await;
    let first = task.client.connections_opened();
    let submit = BusRequest::new(Some("acct-main"), "message", "message.submit", json!({}))
        .with_operation_id("two-reconnects".to_owned());

    // N is lost and its disconnect is processed before anything reconnects.
    fake.shutdown();
    let lost = next_state(&mut inbox, first, false).await;
    task.connection_changed(lost);
    assert!(task.transport_disconnected);

    // N+1 opens (a keyed resend) and its connect notification is processed:
    // exactly one recovery for that generation.
    let b = daemon(Status::Field(json!(0)));
    let _teardown_b = Teardown(b.clone());
    let (fake, _) = start("gate-two-reconnects", &b);
    let epoch = task.gate.epoch();
    let _ = task.client.call(&submit).await;
    assert_eq!(task.client.connections_opened(), first + 1);
    let connected = next_state(&mut inbox, first + 1, true).await;
    task.connection_changed(connected);
    assert_eq!(task.gate.epoch(), epoch + 1, "one recovery decision for connection N+1");
    pump_with(&mut task, &mut inbox, false, |t| {
        verdict(t) == SchemaGate::Open && t.gate_connection == Some(first + 1) && settled(t)
    })
    .await;
    assert_eq!(task.gate.epoch(), epoch + 1, "and no second one while it settles");

    // N+2 opens before N+1's disconnect is processed. It is a new connection
    // and gets its own epoch; nothing left over from N+1 adopts it.
    fake.shutdown();
    let c = daemon(Status::Field(json!(0)));
    let _teardown_c = Teardown(c.clone());
    let (fake, _) = start("gate-two-reconnects", &c);
    let _ = task.client.call(&submit).await;
    assert_eq!(task.client.connections_opened(), first + 2);
    pump_with(&mut task, &mut inbox, false, |t| {
        t.gate.epoch() > epoch + 1
            && verdict(t) == SchemaGate::Open
            && t.gate_connection == Some(first + 2)
            && settled(t)
    })
    .await;
    assert_eq!(task.gate.epoch(), epoch + 2, "N+2 was recovered in a new epoch, not adopted");
    // The stale disconnect of N+1 and N+2's connect change nothing.
    // (N+1's disconnect may never be reported: its observer is replaced.)
    let mut late = Vec::new();
    while let Ok(state) = inbox.connection_states.try_recv() {
        late.push((state.generation, state.connected));
        task.connection_changed(state);
    }
    assert!(late.contains(&(first + 2, true)), "N+2's connect was reported: {late:?}");
    settle(&mut task, &mut inbox).await;
    assert_eq!(task.gate.epoch(), epoch + 2, "the late notifications start no epoch");
    let outcome = task.client.call(&submit).await;
    assert!(
        !matches!(&outcome, Err(BusError::Daemon { code, .. }) if code == "SchemaGateClosed"),
        "approved on N+2: {outcome:?}"
    );
    assert_eq!(task.guard_violations, 0);
    fake.shutdown();
}

/// What can happen around a connection change, each at an arbitrary moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ev {
    /// A read failed because the connection is gone: the link is lost.
    Failure,
    /// The backoff is over: the scope attaches again.
    Snapshot,
    /// The old connection's disconnect notification is processed.
    Disconnect,
    /// The new connection opened (a resend, or the attach) and its connect
    /// notification is processed.
    Connect,
    /// A status answer that travelled on the old connection.
    StatusOld,
    /// A status answer that travelled on the new connection.
    StatusNew,
}

fn permutations<T: Copy>(items: &[T]) -> Vec<Vec<T>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut all = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let mut rest = items.to_vec();
        rest.remove(index);
        for mut tail in permutations(&rest) {
            tail.insert(0, *item);
            all.push(tail);
        }
    }
    all
}

/// Opens the connection after `old` as a keyed resend would (the old one is
/// dead), unless something already has.
async fn ensure_opened(task: &Task, old: u64) {
    if task.client.connections_opened() == old {
        task.client.drop_connection().await;
        task.client.attach().await.expect("the daemon accepts the new connection");
    }
}

/// Plays one event of the hop `old` -> `old + 1`.
async fn play(task: &mut Task, event: Ev, old: u64) {
    use crate::bus::client::ConnectionState;
    match event {
        Ev::Failure => {
            if task.client.connections_opened() == old {
                task.client.drop_connection().await;
            }
            task.lost();
        }
        Ev::Snapshot => {
            let before = task.client.connections_opened();
            task.snapshot();
            if task.attach_pending {
                // The attach opens the connection on its own; its completion
                // is processed whenever the scope next runs.
                let deadline = std::time::Instant::now() + Duration::from_secs(20);
                while task.client.connections_opened() == before {
                    assert!(std::time::Instant::now() < deadline, "the attach opens a connection");
                    tokio::task::yield_now().await;
                }
            }
        }
        Ev::Disconnect => {
            task.connection_changed(ConnectionState { generation: old, connected: false });
        }
        Ev::Connect => {
            ensure_opened(task, old).await;
            task.connection_changed(ConnectionState { generation: old + 1, connected: true });
        }
        Ev::StatusOld => {
            let epoch = task.gate.epoch();
            task.internal(Internal::SchemaAnswer {
                stamp: Stamp { epoch, attempt: task.read_attempt },
                generation: old,
                result: Ok(json!({ "schema_generation": 0 })),
            });
        }
        Ev::StatusNew => {
            ensure_opened(task, old).await;
            // The handler's entry check runs before the answer is looked at.
            task.observe_connection();
            let epoch = task.gate.epoch();
            task.internal(Internal::SchemaAnswer {
                stamp: Stamp { epoch, attempt: task.read_attempt },
                generation: old + 1,
                result: Ok(json!({ "schema_generation": 0 })),
            });
        }
    }
}

/// Runs the scope until it is attached, settled, open and has recovered the
/// newest connection.
async fn quiesce(task: &mut Task, inbox: &mut Inbox) {
    task.tick();
    pump_with(task, inbox, false, |t| {
        t.link == Link::Live
            && settled(t)
            && !t.attach_pending
            && verdict(t) == SchemaGate::Open
            && t.recovered_generation == t.client.connections_opened()
    })
    .await;
}

/// One ordering of the events of `old -> old + 1`, then (when `second` is
/// given) the events of `old + 1 -> old + 2`, either after the first hop has
/// settled or straight away. Settled: every connection generation is
/// recovered exactly once. Overlapping: a generation superseded before any
/// check saw it is never recovered, the rest once each, in order, and the
/// newest is never lost. Either way nothing message-class travels on a
/// connection before that connection's own status.
async fn play_orderings(name: &str, hop1: &[Ev], second: Option<&[Ev]>, overlap: bool) {
    let shared = daemon(Status::Field(json!(0)));
    let (fake, path) = start(name, &shared);
    let (mut task, mut inbox, _) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    attach(&mut task, &mut inbox).await;
    quiesce(&mut task, &mut inbox).await;
    let first = task.client.connections_opened();
    assert_eq!(task.recoveries, [first], "the first attach is recovered once");
    for event in hop1 {
        play(&mut task, *event, first).await;
    }
    let mut expected = vec![first, first + 1];
    if !overlap {
        quiesce(&mut task, &mut inbox).await;
        assert_eq!(task.recoveries, expected, "{hop1:?}: one recovery per generation");
    }
    if let Some(hop2) = second {
        for event in hop2 {
            play(&mut task, *event, first + 1).await;
        }
        quiesce(&mut task, &mut inbox).await;
        expected.push(first + 2);
        if !overlap {
            assert_eq!(task.recoveries, expected, "{hop1:?} then {hop2:?}: one recovery each");
        }
    }
    // Whatever notifications are still queued change nothing.
    let recovered = task.recoveries.len();
    while let Ok(state) = inbox.connection_states.try_recv() {
        task.connection_changed(state);
    }
    quiesce(&mut task, &mut inbox).await;
    assert_eq!(task.recoveries.len(), recovered, "late notifications decide nothing: {hop1:?}");
    // No lost generation, and the last connection carries messages.
    let last = task.client.connections_opened();
    assert_eq!(task.recovered_generation, last);
    assert!(task.client.approval().permits("message.submit", last));
    assert_eq!(task.guard_violations, 0, "no message read left the ordered pipeline: {hop1:?}");
    if overlap {
        let log = &task.recoveries;
        assert!(log.windows(2).all(|pair| pair[0] < pair[1]), "in order, once each: {log:?}");
        assert_eq!((log.first(), log.last()), (Some(&first), Some(&last)), "{hop1:?}: {log:?}");
    }
    // Per connection (the fake numbers them in accept order, as the client
    // numbers its generations): a message-class request is preceded by the
    // status read of the same connection.
    let seen = fake.seen();
    // The event stream is a connection of its own (it carries no request of
    // the gate's); the request connections are the others.
    let connections = seen.iter().map(|r| r.connection).max().unwrap_or(0);
    let mut newest_request = 0;
    for connection in 1..=connections {
        let on: Vec<&str> =
            seen.iter().filter(|r| r.connection == connection).map(|r| r.method.as_str()).collect();
        if on.iter().any(|m| *m != "bus.subscribe") {
            newest_request = connection;
            // Settled hops: every request generation was recovered, so every
            // one carried messages. (An overlapping hop may supersede a
            // generation before anything was sent on it.)
            if !overlap {
                assert!(
                    on.iter().any(|m| gate::is_message_method(m) || *m == "conversation.directory"),
                    "settled connection {connection} carried messages: {on:?} ({hop1:?})"
                );
            }
        }
        let status = on.iter().position(|m| *m == "daemon.status");
        if let Some(message) = on.iter().position(|m| gate::is_message_method(m)) {
            let status = status.unwrap_or_else(|| {
                panic!("connection {connection} carried messages without a status: {on:?}")
            });
            assert!(status < message, "connection {connection}: status first: {on:?} ({hop1:?})");
        }
    }
    let on_newest: Vec<&str> =
        seen.iter().filter(|r| r.connection == newest_request).map(|r| r.method.as_str()).collect();
    assert!(
        on_newest.iter().any(|m| gate::is_message_method(m)),
        "the newest request connection carries messages: {on_newest:?}"
    );
    fake.shutdown();
}

fn orderings() -> Vec<Vec<Ev>> {
    permutations(&[
        Ev::Failure,
        Ev::Snapshot,
        Ev::Disconnect,
        Ev::Connect,
        Ev::StatusOld,
        Ev::StatusNew,
    ])
    .into_iter()
    .filter(|order| {
        // The backoff attach only follows the failure that lost the link.
        let at = |wanted| order.iter().position(|e| *e == wanted);
        at(Ev::Failure) < at(Ev::Snapshot)
    })
    .collect()
}

#[tokio::test]
async fn a_delayed_disconnect_after_the_backoff_attach_recovers_the_new_connection_once() {
    // The reviewer's sequence: the read fails, the backoff attaches before the
    // disconnect is processed, the new connection's status is answered
    // before its connect notification, and the late disconnect and connect
    // arrive last.
    play_orderings(
        "gate-delayed-disconnect",
        &[Ev::Failure, Ev::Snapshot, Ev::StatusNew, Ev::Disconnect, Ev::Connect, Ev::StatusOld],
        None,
        false,
    )
    .await;
}

#[tokio::test]
async fn every_ordering_of_a_connection_change_recovers_each_generation_exactly_once() {
    let all = orderings();
    assert_eq!(all.len(), 360, "all orderings with the failure before the attach");
    for order in &all {
        play_orderings("gate-orderings", order, None, false).await;
    }
}

#[tokio::test]
async fn every_ordering_of_two_consecutive_connection_changes_recovers_each_generation_once() {
    let all = orderings();
    // The second hop (the new connection lost, a third one opened by a
    // resend) takes every interleaving of its four events, rotating through
    // them across the first hop's orderings.
    let second = permutations(&[Ev::Disconnect, Ev::Connect, Ev::StatusOld, Ev::StatusNew]);
    assert_eq!(second.len(), 24);
    for (index, order) in all.iter().enumerate() {
        play_orderings("gate-orderings-two", order, Some(&second[index % second.len()]), false)
            .await;
    }
}

#[tokio::test]
async fn a_second_hop_that_starts_before_the_first_settles_still_recovers_in_order() {
    // Not quiesced between the hops: the first hop's attach and status reads
    // are still outstanding when the next connection opens.
    let all = orderings();
    let second = permutations(&[Ev::Disconnect, Ev::Connect, Ev::StatusOld, Ev::StatusNew]);
    for (index, order) in all.iter().enumerate().filter(|(index, _)| index % 6 == 0) {
        play_orderings("gate-overlap", order, Some(&second[index % second.len()]), true).await;
    }
}

#[tokio::test]
async fn nothing_message_class_is_sent_before_the_first_attach_completes() {
    let shared = daemon(Status::Field(json!(0)));
    let (fake, path) = start("gate-fail-closed", &shared);
    let (mut task, mut inbox, _) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    // Built, not attached: the default is closed, never open.
    assert_eq!(verdict(&task), SchemaGate::Unresolved);
    let start_effect = |task: &mut Task| {
        task.effect(Effect::Call {
            method: "conversation.start".to_owned(),
            body: json!({ "context_id": "c-9", "participant_id": "principal-ana", "text": "hi" }),
        });
    };
    start_effect(&mut task);
    // The attach is requested; its completion has not been handled yet.
    task.snapshot();
    assert!(task.attach_pending, "the connection is still being opened");
    assert_eq!(verdict(&task), SchemaGate::Unresolved);
    start_effect(&mut task);
    let submit = BusRequest::new(Some("acct-main"), "message", "message.submit", json!({}))
        .with_operation_id("before-attach".to_owned());
    let outcome = task.client.call(&submit).await;
    assert!(
        matches!(&outcome, Err(BusError::Daemon { code, .. }) if code == "SchemaGateClosed"),
        "the wire refuses before any schema check: {outcome:?}"
    );
    assert_eq!(message_requests(&fake), Vec::<String>::new(), "zero message-class frames");
    settle(&mut task, &mut inbox).await;
    assert_eq!(verdict(&task), SchemaGate::Open);
    fake.shutdown();
}

#[tokio::test]
async fn a_snapshot_retry_on_a_healthy_connection_holds_messages_behind_the_identities() {
    let shared = daemon(Status::Field(json!(0)));
    let _teardown_shared = Teardown(shared.clone());
    let (fake, path) = start("gate-healthy-retry", &shared);
    let (mut task, mut inbox, _) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    attach(&mut task, &mut inbox).await;
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    settle(&mut task, &mut inbox).await;
    let (before, generation, epoch) =
        (fake.seen().len(), task.client.connections_opened(), task.gate.epoch());
    // The job link failed (the ordinary connection is healthy and approved):
    // the scope attaches again, with the identities held.
    shared.identities.close();
    task.lost();
    task.snapshot();
    assert_eq!(task.client.connections_opened(), generation, "the same connection");
    assert_eq!(task.gate.epoch(), epoch, "no new epoch: it is a retry");
    // The daemon has the identity request in hand (its latch is held).
    pump(&mut task, &mut inbox, |t| {
        t.gate.prerequisites_pending()
            && fake.seen()[before..].iter().any(|r| r.method == "identity.directory")
    })
    .await;
    let held: Vec<String> = fake.seen()[before..].iter().map(|r| r.method.clone()).collect();
    assert!(held.iter().any(|m| m == "identity.directory"), "the prerequisites were asked");
    assert!(
        !held.iter().any(|m| gate::is_message_method(m) || m == "conversation.directory"),
        "no message read beside the held identities: {held:?}"
    );
    assert_eq!(task.guard_violations, 0);
    shared.identities.release();
    settle(&mut task, &mut inbox).await;
    let order: Vec<String> = fake.seen()[before..].iter().map(|r| r.method.clone()).collect();
    let identities = order.iter().position(|m| m == "identity.directory").expect("identities");
    let messages = order.iter().position(|m| m == "conversation.directory").expect("messages");
    assert!(identities < messages, "identities first: {order:?}");
    assert_eq!(task.guard_violations, 0);
    fake.shutdown();
}

#[tokio::test]
async fn a_stale_batch_completion_cannot_release_the_barrier_of_a_retry() {
    let shared = daemon(Status::Field(json!(0)));
    let _teardown_shared = Teardown(shared.clone());
    let (fake, path) = start("gate-stale-batch", &shared);
    let (mut task, mut inbox, _) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    attach(&mut task, &mut inbox).await;
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    settle(&mut task, &mut inbox).await;
    let epoch = task.gate.epoch();

    // A successful snapshot batch is queued (its answers are all in) and
    // the scope has not looked at it yet.
    task.lost();
    task.snapshot();
    // Collect, without dispatching, until the batch's own completion has
    // arrived.
    let mut queued = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let item = tokio::time::timeout(left, inbox.internal.recv())
            .await
            .expect("the snapshot completion arrives")
            .expect("the inbox is open");
        let done = matches!(item, Internal::Snapshot(..));
        queued.push(item);
        if done {
            break;
        }
    }
    assert!(queued.iter().any(|i| matches!(i, Internal::Snapshot(..))), "a batch is queued");

    // The job link fails and the backoff is due: the retry reads again in
    // the same epoch, with the identities held.
    let before = fake.seen().len();
    shared.identities.close();
    task.lost();
    task.snapshot();
    assert_eq!(task.gate.epoch(), epoch, "same epoch: it is a retry");
    pump_with(&mut task, &mut inbox, false, |t| {
        t.gate.prerequisites_pending()
            && fake.seen()[before..].iter().any(|r| r.method == "identity.directory")
    })
    .await;

    // Only now the old attempt's completion is looked at.
    for item in queued {
        task.internal(item);
    }
    assert!(task.gate.prerequisites_pending(), "the barrier still stands");
    assert_eq!(task.batch_pending, 1, "the old completion did not count");
    let held: Vec<String> = fake.seen()[before..].iter().map(|r| r.method.clone()).collect();
    assert!(
        !held.iter().any(|m| gate::is_message_method(m) || m == "conversation.directory"),
        "no message read beside the held identities: {held:?}"
    );
    assert_eq!(task.guard_violations, 0);
    shared.identities.release();
    settle(&mut task, &mut inbox).await;
    let order: Vec<String> = fake.seen()[before..].iter().map(|r| r.method.clone()).collect();
    let identities = order.iter().position(|m| m == "identity.directory").expect("identities");
    let messages = order.iter().position(|m| m == "conversation.directory").expect("messages");
    assert!(identities < messages, "identities first: {order:?}");
    fake.shutdown();
}

#[tokio::test]
async fn a_stale_single_read_error_cannot_lock_or_cancel_a_retry() {
    let shared = daemon(Status::Field(json!(0)));
    let _teardown_shared = Teardown(shared.clone());
    let (fake, path) = start("gate-stale-single", &shared);
    let (mut task, mut inbox, _) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    attach(&mut task, &mut inbox).await;
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    settle(&mut task, &mut inbox).await;
    let epoch = task.gate.epoch();

    // An account.status completion of the current attempt is on its way and
    // says the account is not open.
    let stale = Internal::FetchedAt {
        stamp: task.stamp(),
        read: (
            Fetch::AccountStatus(AccountId::new("acct-main")),
            None,
            Err(BusError::daemon("AccountNotOpen", "account not open: acct-main")),
        ),
    };

    // The job link fails and the backoff retry reads again in the same
    // epoch (the account is open again), identities held.
    let before = fake.seen().len();
    shared.identities.close();
    task.lost();
    task.snapshot();
    assert_eq!(task.gate.epoch(), epoch, "same epoch: it is a retry");
    pump_with(&mut task, &mut inbox, false, |t| {
        t.gate.prerequisites_pending()
            && fake.seen()[before..].iter().any(|r| r.method == "identity.directory")
    })
    .await;
    assert!(task.snapshot_task.is_some(), "the retry batch is running");

    task.internal(stale);
    assert_ne!(task.link, Link::Locked, "a stale error does not lock the recovered scope");
    assert!(task.snapshot_task.is_some(), "the retry batch was not cancelled");
    assert!(task.gate.prerequisites_pending(), "the barrier stands");
    assert_eq!(task.batch_pending, 1);
    let held: Vec<String> = fake.seen()[before..].iter().map(|r| r.method.clone()).collect();
    assert!(
        !held.iter().any(|m| gate::is_message_method(m) || m == "conversation.directory"),
        "no message read beside the held identities: {held:?}"
    );
    shared.identities.release();
    settle(&mut task, &mut inbox).await;
    assert_eq!(task.link, Link::Live);
    let order: Vec<String> = fake.seen()[before..].iter().map(|r| r.method.clone()).collect();
    let identities = order.iter().position(|m| m == "identity.directory").expect("identities");
    let messages = order.iter().position(|m| m == "conversation.directory").expect("messages");
    assert!(identities < messages, "identities first: {order:?}");
    fake.shutdown();
}

#[tokio::test]
async fn reconnect_revokes_the_approval_and_a_late_old_epoch_answer_cannot_reopen() {
    let shared = daemon(Status::Field(json!(0)));
    let _teardown_shared = Teardown(shared.clone());
    let (fake, path) = start("gate-reconnect", &shared);
    let (mut task, mut inbox, delivered) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    attach(&mut task, &mut inbox).await;
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
    let _teardown_restarted = Teardown(restarted.clone());
    restarted.hold.close();
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
        stamp: Stamp { epoch: first_epoch, attempt: task.read_attempt },
        generation: task.client.connections_opened(),
        result: Ok(json!({ "schema_generation": 0 })),
    });
    assert_eq!(verdict(&task), SchemaGate::Unresolved, "a late old-epoch answer is ignored");
    restarted.hold.release();
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    fake.shutdown();
}

#[tokio::test]
async fn an_older_snapshot_arriving_after_the_new_epoch_cannot_reopen_the_gate() {
    let shared = daemon(Status::Field(json!(0)));
    let (fake, path) = start("gate-old-snapshot", &shared);
    let (mut task, mut inbox, delivered) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    attach(&mut task, &mut inbox).await;
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    settle(&mut task, &mut inbox).await;
    let old_epoch = task.gate.epoch();
    // The daemon now reports an incompatible generation; a new epoch asks.
    *shared.status.lock().expect("status") = Status::Field(json!(2));
    // A new epoch on the same connection (the status is read again).
    task.recover_connection();
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Unsupported).await;
    settle(&mut task, &mut inbox).await;
    // The old epoch's snapshot (status 0, and a capability it alone
    // reports) completes only now: nothing of it is translated.
    delivered.try_iter().for_each(drop);
    task.internal(Internal::Snapshot(
        Stamp { epoch: old_epoch, attempt: task.read_attempt },
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
    task.internal(task.fetched_now((
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
    attach(&mut task, &mut inbox).await;
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
    task.internal(task.fetched_now((
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
    let _teardown_shared = Teardown(shared.clone());
    shared.identities.close();
    let (fake, path) = start("gate-order", &shared);
    let (mut task, mut inbox, _) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    attach(&mut task, &mut inbox).await;
    // The status is in and the gate open, but the identity read is held.
    pump(&mut task, &mut inbox, |t| {
        verdict(t) == SchemaGate::Open && t.gate.prerequisites_pending()
    })
    .await;
    assert_eq!(message_requests(&fake), Vec::<String>::new(), "no message read before identities");
    shared.identities.release();
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
    let _teardown_shared = Teardown(shared.clone());
    shared.hold.close();
    let (fake, path) = start("gate-accounts", &shared);
    let (mut first, mut first_inbox, _) = account_task(&path, "acct-one", SchemaSupport::Legacy);
    let (mut second, mut second_inbox, _) = account_task(&path, "acct-two", SchemaSupport::Legacy);
    attach(&mut first, &mut first_inbox).await;
    attach(&mut second, &mut second_inbox).await;
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
    attach(&mut task, &mut inbox).await;
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
    attach(&mut task, &mut inbox).await;
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    settle(&mut task, &mut inbox).await;
    let old_epoch = task.gate.epoch();
    // A group receive is outstanding when the connection changes: a new
    // epoch begins, closes on an incompatible status, and reopens.
    *shared.status.lock().expect("status") = Status::Field(json!(2));
    // A new epoch on the same connection (the status is read again).
    task.recover_connection();
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
        stamp: Stamp { epoch: old_epoch, attempt: task.read_attempt },
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
    let _teardown_shared = Teardown(shared.clone());
    let (fake, path) = start("gate-recover-order", &shared);
    let (mut task, mut inbox, _) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    attach(&mut task, &mut inbox).await;
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    settle(&mut task, &mut inbox).await;
    let known = task.gate_connection;
    fake.shutdown();

    // The daemon restarts with the identity directory held. A keyed resend
    // reconnects; the connection notification starts the recovery.
    let restarted = daemon(Status::Field(json!(0)));
    let _teardown_restarted = Teardown(restarted.clone());
    restarted.identities.close();
    let (fake, _) = start("gate-recover-order", &restarted);
    let submit = BusRequest::new(Some("acct-main"), "message", "message.submit", json!({}))
        .with_operation_id("recover-key".to_owned());
    let _ = task.client.call(&submit).await;
    // The status resolves open, but the identities have not arrived: the
    // messages stay behind them.
    pump(&mut task, &mut inbox, |t| {
        verdict(t) == SchemaGate::Open
            && t.gate_connection > known
            && t.gate.prerequisites_pending()
    })
    .await;
    assert_eq!(
        message_requests(&fake).iter().filter(|m| *m != "message.submit").count(),
        0,
        "no message read is released while the identities are held"
    );
    restarted.identities.release();
    settle(&mut task, &mut inbox).await;
    let order: Vec<String> = fake.seen().iter().map(|r| r.method.clone()).collect();
    let identities = order.iter().position(|m| m == "identity.directory").expect("identities");
    let messages = order.iter().position(|m| m == "conversation.directory").expect("messages");
    assert!(identities < messages, "identities were read first: {order:?}");
    fake.shutdown();
}

#[tokio::test]
async fn a_reconnect_while_the_status_is_unresolved_recovers_in_a_new_epoch() {
    // Epoch on connection N: its prerequisite batch finishes (the snapshot's
    // own status read is answered) while the gate's schema status stays
    // outstanding.
    let shared = daemon(Status::Field(json!(0)));
    let _teardown_shared = Teardown(shared.clone());
    // Every status stays outstanding (the gate's own and the snapshot's):
    // which of them the daemon sees first is not under the test's control.
    shared.hold.close();
    let (fake, path) = start("gate-unresolved-reconnect", &shared);
    let (mut task, mut inbox, _) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    attach(&mut task, &mut inbox).await;
    // Both status reads have reached the daemon, and neither is answered.
    pump(&mut task, &mut inbox, |_| {
        fake.seen().iter().filter(|r| r.method == "daemon.status").count() >= 2
    })
    .await;
    assert_eq!(verdict(&task), SchemaGate::Unresolved);
    let first_epoch = task.gate.epoch();
    assert_eq!(task.gate_connection, None, "no status has resolved on N");
    fake.shutdown();

    // The daemon restarts with changed identities held; a keyed resend opens
    // N+1 before N's disconnect (or N's failing status) is processed.
    let restarted = daemon(Status::Field(json!(0)));
    let _teardown_restarted = Teardown(restarted.clone());
    restarted.identities.close();
    let (fake, _) = start("gate-unresolved-reconnect", &restarted);
    let submit = BusRequest::new(Some("acct-main"), "message", "message.submit", json!({}))
        .with_operation_id("unresolved-key".to_owned());
    let _ = task.client.call(&submit).await;
    pump(&mut task, &mut inbox, |t| {
        t.gate.epoch() > first_epoch
            && verdict(t) == SchemaGate::Open
            && t.gate.prerequisites_pending()
    })
    .await;
    // N's failing status answer (and its retry timer) cannot approve N+1 for
    // the old epoch, and nothing message-class goes ahead of the held
    // prerequisites.
    assert_eq!(message_requests(&fake), Vec::<String>::new(), "messages wait for the identities");
    restarted.identities.release();
    settle(&mut task, &mut inbox).await;
    let order: Vec<String> = fake.seen().iter().map(|r| r.method.clone()).collect();
    let identities = order.iter().position(|m| m == "identity.directory").expect("identities");
    let messages = order.iter().position(|m| m == "conversation.directory").expect("messages");
    assert!(identities < messages, "identities were read first: {order:?}");
    assert_eq!(task.guard_violations, 0);
    fake.shutdown();
}

#[tokio::test]
async fn a_subscription_resync_reads_the_prerequisites_before_the_messages() {
    let shared = daemon(Status::Field(json!(0)));
    let _teardown_shared = Teardown(shared.clone());
    let (fake, path) = start("gate-resync", &shared);
    let (mut task, mut inbox, _) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    attach(&mut task, &mut inbox).await;
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    settle(&mut task, &mut inbox).await;
    let before = fake.seen().len();
    // The subscription needs a resync while the request connection stays
    // healthy: the stream reopens and reads everything again.
    shared.identities.close();
    task.sync.reload_on_open = true;
    task.on_streamed(Streamed::Opened { head_seq: 9, floor_seq: 1 });
    pump(&mut task, &mut inbox, |t| {
        t.gate.prerequisites_pending() && verdict(t) == SchemaGate::Open
    })
    .await;
    // A focus read for a newly encountered sender's conversation is held too.
    task.spawn_fetches(vec![Fetch::Messages(ramflux_desktop_core::ContextId::new("c-new"))]);
    let held: Vec<String> = fake.seen()[before..].iter().map(|r| r.method.clone()).collect();
    assert!(held.iter().any(|m| m == "identity.directory"), "the prerequisites were requested");
    assert!(
        !held.iter().any(|m| gate::is_message_method(m)),
        "no message-class read while identities are held: {held:?}"
    );
    shared.identities.release();
    settle(&mut task, &mut inbox).await;
    let order: Vec<String> = fake.seen()[before..].iter().map(|r| r.method.clone()).collect();
    let identities = order.iter().position(|m| m == "identity.directory").expect("identities");
    let messages = order.iter().position(|m| m == "message.list").expect("the held focus read");
    assert!(identities < messages, "identities first: {order:?}");
    assert_eq!(task.guard_violations, 0, "no message read left the ordered pipeline");
    fake.shutdown();
}

/// One non-test bridge module, normalized: comments removed, whitespace
/// collapsed and no space around `.` and `(`, so a call split over lines or
/// spelled with another receiver is still the same text.
struct Module {
    file: String,
    text: String,
}

impl Module {
    fn count(&self, needle: &str) -> usize {
        self.text.matches(needle).count()
    }

    /// `(function name, body)` for every `fn` of the module.
    fn functions(&self) -> Vec<(String, &str)> {
        let bytes = self.text.as_bytes();
        let mut starts = Vec::new();
        let mut at = 0;
        while let Some(found) = self.text[at..].find("fn ") {
            let index = at + found;
            let boundary = index == 0
                || !(bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_');
            if boundary {
                starts.push(index);
            }
            at = index + 3;
        }
        starts
            .iter()
            .enumerate()
            .map(|(n, &start)| {
                let end = starts.get(n + 1).copied().unwrap_or(self.text.len());
                let name: String = self.text[start + 3..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                (name, &self.text[start..end])
            })
            .collect()
    }
}

fn normalize(source: &str) -> String {
    // Cut the trailing test modules (their own files are skipped below).
    let source = source.split("#[cfg(test)]\n#[path").next().unwrap_or(source);
    let mut text = String::new();
    for line in source.lines() {
        text.push_str(line.split("//").next().unwrap_or(""));
        text.push(' ');
    }
    let mut text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    for (from, to) in [(" .", "."), (". ", "."), (" (", "("), ("( ", "("), (" )", ")")] {
        while text.contains(from) {
            text = text.replace(from, to);
        }
    }
    text
}

/// Every module under `bridge/`, found on disk (a new module is covered
/// without being listed); test files are not product code.
fn bridge_modules() -> Vec<Module> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bridge");
    let mut modules = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("read bridge dir") {
        let path = entry.expect("entry").path();
        let file = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_owned();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") || file.ends_with("_tests.rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("read module");
        modules.push(Module { file, text: normalize(&source) });
    }
    modules.sort_by(|a, b| a.file.cmp(&b.file));
    modules
}

#[test]
fn reads_and_requests_leave_the_bridge_only_at_known_sites() {
    // Mechanical guard over every bridge module on disk. A transmission site
    // (`.call(<anything>)`, `call_on_connection(`, `read(&client`) is listed
    // here deliberately; a module that is not listed must have none.
    // Message-class sites are gated at the client; reads go through
    // `spawn_reads` or `run_batch`.
    let known: &[(&str, usize, usize, usize)] = &[
        ("effects.rs", 8, 0, 0),
        ("mod.rs", 1, 0, 0),
        ("scope.rs", 3, 1, 2),
        ("sends.rs", 1, 0, 0),
    ];
    let modules = bridge_modules();
    assert!(modules.iter().any(|m| m.file == "scope.rs"), "the scan found the bridge");
    for module in &modules {
        let (calls, on_connection, reads) = known
            .iter()
            .find(|(file, ..)| *file == module.file)
            .map_or((0, 0, 0), |&(_, c, o, r)| (c, o, r));
        let file = &module.file;
        assert_eq!(module.count(".call("), calls, "{file}: .call( sites");
        assert_eq!(module.count("call_on_connection("), on_connection, "{file}: schema status");
        assert_eq!(module.count("read(&client"), reads, "{file}: read dispatch sites");
    }

    // Recovery goes through the ordered pipeline only: the primitives that
    // start or restart an epoch are reached from nowhere else, and no
    // function re-reads the snapshot's list through the unordered coalescer
    // (whatever the local variable is called).
    let allowed = |name: &str, only: &[&str]| only.contains(&name);
    for module in &modules {
        for (name, body) in module.functions() {
            let file = &module.file;
            if body.contains("cancel_reads(") {
                assert!(
                    allowed(
                        &name,
                        // Teardown (a lost link, a lock, the task ending) only
                        // stops reads; it starts none.
                        &[
                            "snapshot",
                            "attach_live",
                            "recover_connection",
                            "cancel_reads",
                            "lost",
                            "locked",
                            "drop"
                        ]
                    ),
                    "{file}: `{name}` cancels reads outside the recovery routine"
                );
            }
            if body.contains("begin_gate_epoch(") {
                assert!(
                    allowed(&name, &["start_epoch_reads", "begin_gate_epoch"]),
                    "{file}: `{name}` starts an epoch outside the ordered pipeline"
                );
            }
            if body.contains("start_epoch_reads(") {
                assert!(
                    allowed(&name, &["attach_live", "recover_connection", "start_epoch_reads"]),
                    "{file}: `{name}` starts an epoch's reads outside the recovery routine"
                );
            }
            if body.contains("reads_for_snapshot(") {
                assert!(
                    allowed(&name, &["start_epoch_reads", "run_release", "reads_for_snapshot"]),
                    "{file}: `{name}` re-reads the snapshot list outside the ordered pipeline"
                );
            }
            // The two resync/restore entry points end in the one routine.
            if name == "reload" {
                assert!(body.contains("recover_connection("), "{file}: reload must recover");
                assert!(!body.contains("request_reads("), "{file}: reload must not re-read");
            }
            if name == "snapshot" && file == "scope.rs" {
                assert!(body.contains("attach_live("), "snapshot attaches through the one path");
            }
            // Every inbox message says whether it is a read (exhaustively), and
            // the one check at the inbox drops a superseded read first.
            if name == "stamp" && body.contains("Self::Snapshot") {
                assert!(!body.contains("_ =>"), "{file}: Internal::stamp has no wildcard arm");
            }
            if name == "internal" && file == "scope.rs" {
                assert!(
                    body.contains("internal.stamp()") && body.contains("self.stamp()"),
                    "the inbox rejects superseded reads in one shared check"
                );
            }
            if name == "recover_connection" {
                assert!(body.contains("start_epoch_reads("), "recovery starts the ordered epoch");
            }
            // Recovery is a pure function of the generation counter: it is
            // decided only by the check (and by the attach and resync
            // entry points); a notification or a status answer never decides.
            if body.contains("recover_connection(") {
                assert!(
                    allowed(
                        &name,
                        &["observe_connection", "attach_live", "reload", "recover_connection"]
                    ),
                    "{file}: `{name}` decides a recovery; only the generation check may"
                );
            }
            if name == "observe_connection" {
                assert!(
                    body.contains("connections_opened()>self.recovered_generation")
                        || body.contains("connections_opened() > self.recovered_generation"),
                    "the check compares the monotonic counter"
                );
            }
        }
    }
    // The pipeline's own steps: prerequisites are tracked and the release is
    // held behind them.
    let scope = modules.iter().find(|m| m.file == "scope.rs").expect("scope.rs");
    let epoch = scope.functions().into_iter().find(|(n, _)| n == "start_epoch_reads").expect("fn");
    assert!(epoch.1.contains("set_prerequisites("), "the epoch tracks its prerequisite batch");
}
