// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The schema gate on the wire (S10c D1): deterministic held, failed and
//! timed-out status answers against a recording fake daemon. Every row
//! asserts the message-class requests the daemon actually received.
use super::*;
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
}

type Shared = Arc<Daemon>;

fn daemon(status: Status) -> Shared {
    Arc::new(Daemon {
        status: Mutex::new(status),
        hold: AtomicBool::new(false),
        reset: AtomicBool::new(false),
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

async fn pump(task: &mut Task, inbox: &mut Inbox, done: impl Fn(&Task) -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
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

/// Lets the task run for `ms` without waiting for anything.
async fn run_for(task: &mut Task, inbox: &mut Inbox, ms: u64) {
    let until = std::time::Instant::now() + Duration::from_millis(ms);
    pump(task, inbox, |_| std::time::Instant::now() >= until).await;
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
    assert_eq!(task.translator.cached_plaintext(), 1, "the oracle is seeded");
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
    run_for(&mut task, &mut inbox, 400).await;
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
    run_for(&mut task, &mut inbox, 300).await;
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
    run_for(&mut task, &mut inbox, 200).await;
    assert_eq!(message_requests(&fake), Vec::<String>::new(), "failed: zero requests");
    assert!(gates(&delivered).iter().any(|(gate, _)| *gate == SchemaGate::Failed));
    *shared.status.lock().expect("status") = Status::Field(json!(0));
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    run_for(&mut task, &mut inbox, 200).await;
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
            run_for(&mut task, &mut inbox, 200).await;
            assert_eq!(verdict(&task), want, "{name} under {support:?}");
            let sent = message_requests(&fake);
            assert_eq!(!sent.is_empty(), want.is_open(), "{name} under {support:?}: {sent:?}");
            fake.shutdown();
        }
    }
}

#[tokio::test]
async fn reconnect_revokes_the_approval_and_a_late_old_epoch_answer_cannot_reopen() {
    let shared = daemon(Status::Field(json!(0)));
    let (fake, path) = start("gate-reconnect", &shared);
    let (mut task, mut inbox, delivered) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    task.snapshot();
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    run_for(&mut task, &mut inbox, 200).await;
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
    run_for(&mut task, &mut inbox, 300).await;
    assert_eq!(verdict(&task), SchemaGate::Unresolved, "the approval is revoked");
    assert_eq!(message_requests(&fake), Vec::<String>::new(), "zero message-class requests");
    assert_eq!(task.translator.cached_plaintext(), 0, "caches are empty at the data layer");
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
    task.internal(Internal::Fetched((
        Fetch::SchemaStatus(first_epoch),
        None,
        Ok(json!({ "schema_generation": 0 })),
    )));
    assert_eq!(verdict(&task), SchemaGate::Unresolved, "a late old-epoch answer is ignored");
    assert_eq!(message_requests(&fake), Vec::<String>::new());
    restarted.hold.store(false, Ordering::SeqCst);
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    fake.shutdown();
}

#[tokio::test]
async fn a_changed_status_in_the_same_epoch_blocks_at_once_and_sends_nothing() {
    let shared = daemon(Status::Field(json!(0)));
    let (fake, path) = start("gate-change", &shared);
    let (mut task, mut inbox, delivered) = account_task(&path, "acct-main", SchemaSupport::Legacy);
    task.snapshot();
    pump(&mut task, &mut inbox, |t| verdict(t) == SchemaGate::Open).await;
    run_for(&mut task, &mut inbox, 200).await;
    seed_plaintext(&mut task);
    delivered.try_iter().for_each(drop);
    let before = fake.seen().len();
    // The next status read says generation 1: this build cannot open it.
    task.on_read(&Fetch::DaemonStatus, Ok(json!({ "schema_generation": 1 })));
    assert_eq!(verdict(&task), SchemaGate::UpdateApp);
    assert_eq!(task.translator.cached_plaintext(), 0, "translator caches are dropped");
    assert_eq!(gates(&delivered), [(SchemaGate::UpdateApp, Some(1))]);
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
    run_for(&mut task, &mut inbox, 200).await;
    assert_eq!(
        fake.seen()[before..].iter().filter(|r| r.method == "message.list").count(),
        0,
        "blocked reads are dropped"
    );
    // Back to a readable generation: the dropped reads are made again.
    task.on_read(&Fetch::DaemonStatus, Ok(json!({ "schema_generation": 0 })));
    assert_eq!(verdict(&task), SchemaGate::Open);
    run_for(&mut task, &mut inbox, 300).await;
    assert!(!message_requests(&fake).is_empty());
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
    run_for(&mut first, &mut first_inbox, 300).await;
    run_for(&mut second, &mut second_inbox, 100).await;
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
    run_for(&mut task, &mut inbox, 300).await;
    assert!(gates(&delivered).iter().any(|(gate, _)| *gate == SchemaGate::ResetRequired));
    let after = message_requests(&fake).len();
    run_for(&mut task, &mut inbox, 300).await;
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
