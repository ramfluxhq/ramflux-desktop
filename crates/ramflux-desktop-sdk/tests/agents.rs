// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! AI participants against a fake daemon that follows the product's bus
//! contract (DESK-AGENT-01): the owner-attended creation ceremony
//! (`account.create.begin` as the custodian's session, the three words only,
//! the phrase handed over once), the export of the one-shot credentials on a
//! connection that attended the owner's account, each refusal the daemon
//! makes mapped to a typed reason, and the deadlines of the two methods.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use ramflux_desktop_core::{
    AccountId, AccountKind, AgentExportPhase, AgentExportRefusal, CeremonyFailure, CeremonyPhase,
    Effect, FailureCode, Networking, SecretInput, Source, methods,
};
use ramflux_desktop_sdk::SdkHub;
use ramflux_desktop_test::fake_custody::FakeCustody;
use ramflux_desktop_test::mock_ceremony::{DEMO_POSITIONS, demo_word};
use serde_json::json;

const OWNER: &str = "acct-owner";

fn call(method: &str, body: serde_json::Value) -> Effect {
    Effect::Call { method: method.to_owned(), body }
}

fn begin(label: &str, custodian: &str) -> Effect {
    call(
        methods::CUSTODY_BEGIN,
        json!({ "display_name": label, "principal_kind": "headless_ai",
                "custodian_account": custodian }),
    )
}

fn words() -> Vec<SecretInput> {
    DEMO_POSITIONS.iter().map(|p| SecretInput::new(demo_word(usize::from(*p)))).collect()
}

fn confirm(secrets: Vec<SecretInput>) -> Effect {
    Effect::SecretCall { method: methods::CUSTODY_CONFIRM.to_owned(), body: json!({}), secrets }
}

fn export(owner: &str, agent: &str, path: &str) -> Effect {
    call(
        methods::AGENT_EXPORT_CREDENTIALS,
        json!({ "owner_id": owner, "account": agent, "path": path }),
    )
}

/// A daemon with the owner's unlocked, connected account, over `custody`.
fn world(name: &str, custody: &Arc<FakeCustody>) -> (FakeBus, SdkHub, Fed) {
    custody.set_live(true);
    custody.add_account(OWNER, "Me", "owner passphrase", false);
    let path = socket_path(name);
    let fake = FakeBus::start_with(
        &path,
        |method, _, _| match method {
            "daemon.status" => Reply::Ok(
                json!({ "accounts": 1, "socket_path": "/tmp/rfd.sock", "born_v3_creation": true }),
            ),
            _ => fixture(method),
        },
        custody.clone(),
    );
    let hub = hub(&path);
    let mut fed = Fed::new(hub.daemon_source());
    fed.until("listed", |s, _| s.account(&AccountId::new(OWNER)).is_some());
    (fake, hub, fed)
}

fn phase(fed: &Fed) -> Option<CeremonyPhase> {
    fed.store.ceremony().map(|c| c.phase.clone())
}

fn created(fed: &mut Fed) -> String {
    fed.source.submit(begin("Ledger", OWNER));
    fed.until("prepared", |s, _| s.ceremony().is_some_and(|c| c.phase == CeremonyPhase::Prepared));
    fed.source.submit(call(methods::CUSTODY_SHOW_ONCE, json!({})));
    fed.until("shown", |s, _| s.ceremony().is_some_and(|c| c.phase == CeremonyPhase::PhraseReady));
    assert!(fed.source.take_recovery_phrase().is_some());
    fed.source.submit(call(methods::CUSTODY_HIDDEN_CHALLENGE, json!({})));
    fed.until("asked", |s, _| {
        s.ceremony().is_some_and(|c| matches!(c.phase, CeremonyPhase::Challenged { .. }))
    });
    fed.source.submit(confirm(words()));
    fed.until("created", |s, _| {
        s.ceremony().is_some_and(|c| matches!(c.phase, CeremonyPhase::Created(_)))
    });
    let Some(CeremonyPhase::Created(made)) = phase(fed) else { panic!("created") };
    made.local_account_id
}

fn bundle_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rf-agent-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    dir
}

#[test]
fn an_ai_participant_is_created_as_its_custodians_session_with_the_three_words_only() {
    let custody = Arc::new(FakeCustody::new());
    let (fake, hub, mut fed) = world("agent-create", &custody);
    let id = created(&mut fed);
    // The begin named the custodian and was sent as that account's session.
    let sent = fake.seen().into_iter().find(|s| s.method == "account.create.begin").expect("begin");
    assert_eq!(sent.account.as_deref(), Some(OWNER));
    assert_eq!(
        sent.body,
        json!({ "display_name": "Ledger", "principal_kind": "headless_ai",
                "custodian_account": OWNER })
    );
    // The confirm carried the words and no passphrase.
    assert_eq!(custody.last_confirm_fields(), vec!["words".to_owned()]);
    // One connection for the whole ceremony, shared with nothing else.
    let connection = custody.connections_of("account.create.begin");
    assert_eq!(connection.len(), 1);
    assert_eq!(custody.connections_of("account.create.confirm"), connection);
    // The account is an AI participant its creator custodies.
    let account = fed.store.account(&AccountId::new(&id)).expect("listed");
    assert_eq!(account.kind, AccountKind::HeadlessAi);
    assert_eq!(account.networking, Networking::Live);
    assert_eq!(fed.store.custodian_of(&account.id), Some(&AccountId::new(OWNER)));
    // The phrase was handed over once and every typed secret was erased.
    let audit = hub.phrase_audit();
    assert_eq!((audit.issued(), audit.taken(), audit.outstanding()), (1, 1, 0));
    assert_eq!((audit.secrets(), audit.secrets_wiped()), (3, 3));
    // No secret is in any event.
    let all = format!("{:?}", fed.envelopes);
    assert!(!all.contains(demo_word(3)) && !all.contains("daemon-made"), "no secret in events");
    fake.shutdown();
}

#[test]
fn the_daemons_refusals_of_a_custodian_end_the_ceremony_as_a_typed_refusal() {
    // The custodian is locked, pending, not a person, not the session, or
    // not known: the begin is refused whatever the cause.
    type Arrange = fn(&FakeCustody);
    let cases: [(&str, Arrange); 4] = [
        ("locked", |c| c.set_locked(OWNER, true)),
        ("pending", |c| c.set_networking_of(OWNER, false)),
        ("agent", |c| c.add_agent("acct-bot", "Bot", OWNER)),
        ("unknown", |_| {}),
    ];
    for (name, arrange) in cases {
        let custody = Arc::new(FakeCustody::new());
        let (fake, _hub, mut fed) = world(&format!("agent-refuse-{name}"), &custody);
        arrange(&custody);
        let custodian = match name {
            "agent" => "acct-bot",
            "unknown" => "acct-nobody",
            _ => OWNER,
        };
        fed.source.submit(begin("Ledger", custodian));
        fed.until(name, |s, _| s.ceremony().is_some());
        assert_eq!(
            phase(&fed),
            Some(CeremonyPhase::Failed(CeremonyFailure::CustodianRefused)),
            "{name}"
        );
        assert!(fed.store.ceremony().is_some_and(|c| c.ceremony_id.is_empty()), "no ceremony");
        assert_eq!(custody.running(), 0, "{name}: nothing is left running");
        fake.shutdown();
    }
}

#[test]
fn the_ceremony_is_begun_as_the_session_of_the_account_that_custodies() {
    // Whatever person's account custodies, the begin is that account's own
    // session (the daemon refuses a frame that names another).
    let custody = Arc::new(FakeCustody::new());
    let (fake, _hub, mut fed) = world("agent-session", &custody);
    custody.add_account("acct-other", "Other", "x", false);
    fed.source.submit(begin("Ledger", "acct-other"));
    fed.until("prepared", |s, _| s.ceremony().is_some());
    assert_eq!(phase(&fed), Some(CeremonyPhase::Prepared));
    let sent = fake.seen().into_iter().find(|s| s.method == "account.create.begin").expect("begin");
    assert_eq!(sent.account.as_deref(), Some("acct-other"));
    fake.shutdown();
}

#[test]
fn the_credentials_are_exported_once_on_an_attended_connection_to_a_private_file() {
    let custody = Arc::new(FakeCustody::new());
    let (fake, _hub, mut fed) = world("agent-export", &custody);
    let id = created(&mut fed);
    let dir = bundle_dir("export");
    let path = dir.join("ledger.bundle");
    let path_text = path.to_string_lossy().into_owned();
    fed.source.submit(export(OWNER, &id, &path_text));
    fed.until("exported", |s, _| s.agent_export(&AccountId::new(&id)).is_some());
    let record = fed.store.agent_export(&AccountId::new(&id)).expect("record");
    assert_eq!(record.phase, AgentExportPhase::Exported);
    assert_eq!(record.path, path_text);
    // The request is exactly what the daemon takes, as the owner, after the
    // owner was attended on the same connection, and carries no retry key.
    let seen = fake.seen();
    let request =
        seen.iter().find(|s| s.method == "account.export_agent_credentials").expect("export");
    assert_eq!(request.account.as_deref(), Some(OWNER));
    assert_eq!(request.body, json!({ "account": id, "path": path_text }));
    assert_eq!(request.operation_id, None);
    let attend = seen
        .iter()
        .find(|s| {
            s.method == "subscription.open"
                && s.connection == request.connection
                && s.account.as_deref() == Some(OWNER)
        })
        .expect("attended first");
    assert_eq!(attend.body["attended_frontend"], true);
    // The file is private, holds no phrase, and the daemon no longer holds
    // the credentials: the export is one-shot.
    use std::os::unix::fs::PermissionsExt as _;
    let mode = std::fs::metadata(&path).expect("file").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert!(!custody.holds_credentials(&id));
    // Nothing the desktop kept names a credential.
    let all = format!("{:?}", fed.envelopes);
    assert!(!all.contains("fake_bundle") && !all.contains("daemon-made"));
    // A second export is refused: the credentials are gone.
    let again = dir.join("again.bundle");
    fed.source.submit(export(OWNER, &id, &again.to_string_lossy()));
    fed.until("refused", |s, _| {
        s.agent_export(&AccountId::new(&id))
            .is_some_and(|e| matches!(e.phase, AgentExportPhase::Refused(_)))
    });
    let record = fed.store.agent_export(&AccountId::new(&id)).expect("record");
    assert_eq!(record.phase, AgentExportPhase::Refused(AgentExportRefusal::CredentialsGone));
    assert!(!again.exists());
    fake.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn each_refusal_of_an_export_is_a_typed_reason_and_writes_nothing() {
    let custody = Arc::new(FakeCustody::new());
    let (fake, _hub, mut fed) = world("agent-export-refusals", &custody);
    custody.add_agent("acct-bot", "Bot", OWNER);
    custody.set_locked("acct-bot", false);
    custody.add_account("acct-other", "Other", "x", false);
    custody.add_agent("acct-theirs", "Theirs", "acct-other");
    custody.set_locked("acct-theirs", false);
    custody.add_agent("acct-wait", "Wait", OWNER);
    custody.set_locked("acct-wait", false);
    custody.set_networking_of("acct-wait", false);
    fed.until("listed", |s, _| s.account(&AccountId::new("acct-wait")).is_some());
    let dir = bundle_dir("refusals");
    let taken = dir.join("taken.bundle");
    std::fs::write(&taken, b"mine").expect("file");
    let missing = dir.join("no-such-folder").join("x.bundle");
    let ok = dir.join("ok.bundle");
    let cases: [(&str, &str, &str, AgentExportRefusal); 6] = [
        ("file exists", OWNER, "acct-bot", AgentExportRefusal::FileExists),
        ("folder missing", OWNER, "acct-bot", AgentExportRefusal::LocationMissing),
        ("not the custodian", OWNER, "acct-theirs", AgentExportRefusal::NotCustodian),
        ("a person", OWNER, OWNER, AgentExportRefusal::NotCustodian),
        ("not connected", OWNER, "acct-wait", AgentExportRefusal::NotConnected),
        ("owner locked", OWNER, "acct-bot", AgentExportRefusal::NotOwnerSession),
    ];
    let paths = [&taken, &missing, &ok, &ok, &ok, &ok];
    for ((name, owner, agent, expected), path) in cases.into_iter().zip(paths) {
        if name == "owner locked" {
            custody.set_locked(OWNER, true);
        }
        let agent_id = AccountId::new(agent);
        let before = fed.store.failures_total();
        fed.source.submit(export(owner, agent, &path.to_string_lossy()));
        fed.until(name, |s, _| s.failures_total() > before);
        let record = fed.store.agent_export(&agent_id).expect(name);
        assert_eq!(record.phase, AgentExportPhase::Refused(expected), "{name}");
        // The failed effect carries the typed reason's key, never the
        // daemon's English.
        let failure = fed.store.failures().last().expect("failure");
        assert!(failure.message.starts_with("backend.agent.export."), "{name}: {failure:?}");
        assert!(matches!(failure.code, FailureCode::Daemon(_)), "{name}");
    }
    assert_eq!(std::fs::read(&taken).expect("kept"), b"mine", "the file was not replaced");
    assert!(!missing.exists() && !ok.exists(), "nothing was written");
    assert!(custody.exports().is_empty());
    fake.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_export_from_a_session_the_daemon_does_not_attend_is_refused() {
    // The fake attends a connection only when it opened the owner's
    // attended subscription; a request sent on the ordinary connection is
    // the daemon's `not owner session` refusal.
    let custody = Arc::new(FakeCustody::new());
    let (fake, _hub, mut fed) = world("agent-export-session", &custody);
    custody.add_agent("acct-bot", "Bot", OWNER);
    custody.set_locked("acct-bot", false);
    fed.until("listed", |s, _| s.account(&AccountId::new("acct-bot")).is_some());
    let client = ramflux_desktop_sdk::bus::client::BusClient::new(
        socket_path("agent-export-session"),
        Duration::from_secs(2),
        ramflux_desktop_sdk::Deadlines::uniform(Duration::from_secs(3)),
    );
    let request = ramflux_desktop_sdk::bus::BusRequest::new(
        Some(OWNER),
        "account",
        "account.export_agent_credentials",
        json!({ "account": "acct-bot", "path": "/tmp/rf-never.bundle" }),
    );
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("rt");
    let error = runtime.block_on(client.call(&request)).expect_err("refused");
    assert_eq!(
        ramflux_desktop_sdk::translate::agents::export_refusal(&error),
        AgentExportRefusal::NotOwnerSession
    );
    assert!(!std::path::Path::new("/tmp/rf-never.bundle").exists());
    fake.shutdown();
}

#[test]
fn a_daemon_that_reports_the_custodian_is_believed_and_the_listing_shows_kind_and_custodian() {
    let custody = Arc::new(FakeCustody::new());
    let (fake, _hub, mut fed) = world("agent-listing", &custody);
    custody.add_agent("acct-bot", "Bot", OWNER);
    fed.until("listed as an AI participant", |s, _| {
        s.account(&AccountId::new("acct-bot")).is_some_and(|a| a.kind == AccountKind::HeadlessAi)
    });
    // The daemon of this product revision does not name the custodian.
    assert_eq!(fed.store.custodian_of(&AccountId::new("acct-bot")), None);
    custody.report_custodian(true);
    fed.until("custodian reported", |s, _| s.custodian_of(&AccountId::new("acct-bot")).is_some());
    assert_eq!(fed.store.custodian_of(&AccountId::new("acct-bot")), Some(&AccountId::new(OWNER)));
    fake.shutdown();
}

#[test]
fn the_two_methods_wait_like_the_account_ceremonies_they_belong_to() {
    let deadlines = ramflux_desktop_sdk::Deadlines::standard();
    for method in ["account.create.begin", "account.export_agent_credentials", "account.create"] {
        assert_eq!(deadlines.budget(method), deadlines.network, "{method}");
    }
}
