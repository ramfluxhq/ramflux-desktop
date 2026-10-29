// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Account creation and recovery with a recovery phrase against a fake
//! daemon that follows the product's bus contract (DESK-V3-02): the
//! capability read from `daemon.status`, one exclusive connection per
//! ceremony, the phrase moved to the screen's side as a zeroizing buffer
//! (and erased when it is not taken), the typed secrets erased after their
//! request, the daemon's refusals mapped to the ceremony's failures
//! (`CustodyExpired` among them), and lock, restart and unlock of an
//! account the ceremony made.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use common::*;
use ramflux_desktop_core::{
    AccountId, CeremonyFailure, CeremonyPhase, DomainEvent, Effect, FailureCode, LockState,
    Networking, RecoveryPhase, SecretInput, Source, methods,
};
use ramflux_desktop_sdk::SdkHub;
use ramflux_desktop_test::fake_custody::{self, FakeCustody};
use ramflux_desktop_test::mock_ceremony::{DEMO_POSITIONS, demo_word};
use serde_json::json;

const PASSPHRASE: &str = "river stone lantern";

fn call(method: &str, body: serde_json::Value) -> Effect {
    Effect::Call { method: method.to_owned(), body }
}

fn begin(label: &str) -> Effect {
    call(methods::CUSTODY_BEGIN, json!({ "display_name": label, "principal_kind": "human" }))
}

fn right_words() -> Vec<SecretInput> {
    DEMO_POSITIONS
        .iter()
        .map(|p| SecretInput::new(demo_word(usize::from(*p))))
        .chain([SecretInput::new(PASSPHRASE)])
        .collect()
}

fn confirm(secrets: Vec<SecretInput>) -> Effect {
    Effect::SecretCall { method: methods::CUSTODY_CONFIRM.to_owned(), body: json!({}), secrets }
}

/// A fake daemon over `custody` that says whether it offers creation.
fn daemon(
    name: &str,
    custody: &Arc<FakeCustody>,
    offered: &Arc<AtomicBool>,
) -> (FakeBus, std::path::PathBuf) {
    let path = socket_path(name);
    let offered = offered.clone();
    let fake = FakeBus::start_with(
        &path,
        move |method, _, _| match method {
            "daemon.status" => Reply::Ok(if offered.load(Ordering::SeqCst) {
                json!({ "accounts": 0, "socket_path": "/tmp/rfd.sock", "born_v3_creation": true })
            } else {
                json!({ "accounts": 0, "socket_path": "/tmp/rfd.sock" })
            }),
            "account.list" => Reply::Ok(json!({ "accounts": [], "account_details": [] })),
            _ => fixture(method),
        },
        custody.clone(),
    );
    (fake, path)
}

fn offering(name: &str) -> (FakeBus, SdkHub, Fed, Arc<FakeCustody>) {
    let custody = Arc::new(FakeCustody::new());
    let (fake, path) = daemon(name, &custody, &Arc::new(AtomicBool::new(true)));
    let hub = hub(&path);
    let mut fed = Fed::new(hub.daemon_source());
    fed.until("creation offered", |s, _| s.offers_born_v3_creation());
    (fake, hub, fed, custody)
}

fn phase(fed: &Fed) -> Option<CeremonyPhase> {
    fed.store.ceremony().map(|c| c.phase.clone())
}

/// Through begin, show and hide (the phrase taken or not).
fn challenged(fed: &mut Fed, take: bool) {
    fed.source.submit(begin("Alex"));
    fed.until("prepared", |s, _| s.ceremony().is_some_and(|c| c.phase == CeremonyPhase::Prepared));
    fed.source.submit(call(methods::CUSTODY_SHOW_ONCE, json!({})));
    fed.until("phrase ready", |s, _| {
        s.ceremony().is_some_and(|c| c.phase == CeremonyPhase::PhraseReady)
    });
    if take {
        assert!(fed.source.take_recovery_phrase().is_some());
    }
    fed.source.submit(call(methods::CUSTODY_HIDDEN_CHALLENGE, json!({})));
    fed.until("challenged", |s, _| {
        s.ceremony().is_some_and(|c| matches!(c.phase, CeremonyPhase::Challenged { .. }))
    });
}

#[test]
fn creation_is_offered_only_when_the_daemon_status_says_so() {
    let custody = Arc::new(FakeCustody::new());
    let offered = Arc::new(AtomicBool::new(false));
    let (fake, path) = daemon("cap", &custody, &offered);
    let hub = hub(&path);
    let mut fed = Fed::new(hub.daemon_source());
    fed.until("status read", |s, _| s.daemon().is_some());
    assert!(!fed.store.offers_born_v3_creation(), "no field: an older daemon");
    offered.store(true, Ordering::SeqCst);
    fed.until("offered", |s, _| s.offers_born_v3_creation());
    offered.store(false, Ordering::SeqCst);
    fed.until("withdrawn", |s, _| !s.offers_born_v3_creation());
    fake.shutdown();
}

#[test]
fn a_creation_runs_on_one_exclusive_connection_and_hands_the_phrase_over_once() {
    let (fake, hub, mut fed, custody) = offering("create");
    let audit = hub.phrase_audit();
    fed.source.submit(begin("Alex"));
    fed.until("prepared", |s, _| s.ceremony().is_some_and(|c| c.phase == CeremonyPhase::Prepared));
    let ceremony = fed.store.ceremony().expect("ceremony").clone();
    assert!(ceremony.ceremony_id.starts_with("ceremony-"));
    assert_eq!(ceremony.label, "Alex");
    // The daemon's remaining time became the expiry, a real clock apart.
    let now = ramflux_desktop_core::Timestamp(
        i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_millis(),
        )
        .expect("time"),
    );
    let left = ceremony.remaining_ms(now);
    assert!((290_000..=300_000).contains(&left), "five minutes from the answer: {left}");
    // The begin carries no secret and nothing the daemon allocates.
    let sent = fake.seen().into_iter().find(|s| s.method == "account.create").expect("begin");
    assert_eq!(sent.body, json!({ "display_name": "Alex", "principal_kind": "human" }));

    assert!(fed.source.take_recovery_phrase().is_none(), "nothing shown yet");
    fed.source.submit(call(methods::CUSTODY_SHOW_ONCE, json!({})));
    fed.until("phrase ready", |s, _| {
        s.ceremony().is_some_and(|c| c.phase == CeremonyPhase::PhraseReady)
    });
    let phrase = fed.source.take_recovery_phrase().expect("the phrase");
    assert_eq!(phrase.words().count(), 24);
    assert_eq!(phrase.word(3), Some(demo_word(3)));
    assert!(fed.source.take_recovery_phrase().is_none(), "given out once");
    drop(phrase);
    assert_eq!((audit.issued(), audit.taken(), audit.wiped(), audit.outstanding()), (1, 1, 0, 0));

    fed.source.submit(call(methods::CUSTODY_HIDDEN_CHALLENGE, json!({})));
    fed.until("challenged", |s, _| {
        s.ceremony()
            .is_some_and(|c| c.phase == CeremonyPhase::Challenged { positions: DEMO_POSITIONS })
    });
    fed.source.submit(confirm(right_words()));
    fed.until("created", |s, _| {
        s.ceremony().is_some_and(|c| matches!(c.phase, CeremonyPhase::Created(_)))
    });
    let Some(CeremonyPhase::Created(account)) = phase(&fed) else { panic!("created") };
    assert_eq!(account.display_name, "Alex");
    assert_eq!(account.networking, Networking::Pending, "the typed networking state");
    let listed = fed.store.account(&AccountId::new(&account.local_account_id)).expect("listed");
    assert_eq!(listed.lock_state, LockState::Unlocked);
    assert_eq!(listed.networking, Networking::Pending);
    assert_eq!(listed.label, "Alex");

    // Exclusive connection: all five steps on one, shared with no other
    // request.
    let connection = custody.connections_of("account.create");
    assert_eq!(connection.len(), 1);
    for method in
        ["account.create.show_once", "account.create.hidden_challenge", "account.create.confirm"]
    {
        assert_eq!(custody.connections_of(method), connection, "{method}");
    }
    assert!(
        fake.seen()
            .iter()
            .filter(|s| !s.method.starts_with("account.create"))
            .all(|s| s.connection != connection[0]),
        "no ordinary request shares the ceremony's connection"
    );
    // What the daemon received was typed, and every copy made for it was erased.
    let (words, passphrase) = custody.last_confirm().expect("confirm");
    assert_eq!(words, DEMO_POSITIONS.map(|p| demo_word(usize::from(p)).to_owned()));
    assert_eq!(passphrase, PASSPHRASE);
    assert_eq!((audit.secrets(), audit.secrets_wiped()), (4, 4));
    // Neither the phrase nor a secret is in any event.
    let all = format!("{:?}", fed.envelopes);
    assert!(!all.contains(PASSPHRASE) && !all.contains(demo_word(3)), "no secret in events");
    fake.shutdown();
}

#[test]
fn a_phrase_the_screen_never_takes_is_erased_when_the_ceremony_moves_on() {
    let (fake, hub, mut fed, _) = offering("untaken");
    let audit = hub.phrase_audit();
    challenged(&mut fed, false);
    assert!(fed.source.take_recovery_phrase().is_none(), "the hide made it unavailable");
    assert_eq!((audit.issued(), audit.taken(), audit.wiped()), (1, 0, 1));
    assert_eq!(audit.outstanding(), 0);
    fake.shutdown();
}

#[test]
fn an_earlier_ceremonys_phrase_is_never_given_to_the_next() {
    let (fake, hub, mut fed, custody) = offering("stale");
    let audit = hub.phrase_audit();
    fed.source.submit(begin("Alex"));
    fed.until("prepared", |s, _| s.ceremony().is_some_and(|c| c.phase == CeremonyPhase::Prepared));
    fed.source.submit(call(methods::CUSTODY_SHOW_ONCE, json!({})));
    fed.until("phrase ready", |s, _| {
        s.ceremony().is_some_and(|c| c.phase == CeremonyPhase::PhraseReady)
    });
    // Cancelled without the screen taking the words.
    fed.source.submit(call(methods::CUSTODY_CANCEL, json!({})));
    fed.until("cancelled", |s, _| {
        s.ceremony().is_some_and(|c| c.phase == CeremonyPhase::Cancelled)
    });
    assert_eq!(custody.running(), 0, "the daemon let go of it");
    assert!(fed.source.take_recovery_phrase().is_none());
    assert_eq!((audit.issued(), audit.wiped()), (1, 1));
    fed.source.submit(begin("Bea"));
    fed.until("second", |s, _| {
        s.ceremony().is_some_and(|c| c.phase == CeremonyPhase::Prepared && c.label == "Bea")
    });
    assert!(fed.source.take_recovery_phrase().is_none(), "nothing from before");
    fed.source.submit(call(methods::CUSTODY_SHOW_ONCE, json!({})));
    fed.until("second phrase", |s, _| {
        s.ceremony().is_some_and(|c| c.phase == CeremonyPhase::PhraseReady)
    });
    assert!(fed.source.take_recovery_phrase().is_some());
    assert_eq!((audit.issued(), audit.taken(), audit.wiped()), (2, 1, 1));
    fake.shutdown();
}

#[test]
fn letting_go_of_the_source_ends_the_ceremony_and_erases_the_phrase() {
    let (fake, hub, mut fed, custody) = offering("dropped");
    let audit = hub.phrase_audit();
    fed.source.submit(begin("Alex"));
    fed.until("prepared", |s, _| s.ceremony().is_some());
    fed.source.submit(call(methods::CUSTODY_SHOW_ONCE, json!({})));
    fed.until("phrase ready", |s, _| {
        s.ceremony().is_some_and(|c| c.phase == CeremonyPhase::PhraseReady)
    });
    assert_eq!(custody.running(), 1);
    drop(fed);
    let limit = std::time::Instant::now() + Duration::from_secs(10);
    while custody.running() > 0 || audit.outstanding() > 0 {
        assert!(std::time::Instant::now() < limit, "the daemon still holds the ceremony");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!((audit.issued(), audit.taken(), audit.wiped()), (1, 0, 1));
    fake.shutdown();
}

#[test]
fn the_daemons_refusals_become_the_ceremonys_failures() {
    // Expired: the daemon's own clock ended it (CustodyExpired).
    let (fake, _hub, mut fed, custody) = offering("expired");
    custody.set_ttl(Duration::from_millis(120));
    fed.source.submit(begin("Alex"));
    fed.until("prepared", |s, _| s.ceremony().is_some());
    std::thread::sleep(Duration::from_millis(250));
    fed.source.submit(call(methods::CUSTODY_SHOW_ONCE, json!({})));
    fed.until("expired", |s, _| {
        s.ceremony().is_some_and(|c| c.phase == CeremonyPhase::Failed(CeremonyFailure::Expired))
    });
    assert!(fed.store.ceremony().is_some_and(|c| !c.ceremony_id.is_empty()), "it had begun");
    assert_eq!(custody.running(), 0);
    fake.shutdown();

    // Vault: begin is refused before a ceremony exists.
    let (fake, _hub, mut fed, custody) = offering("vault");
    custody.set_vault_available(false);
    fed.source.submit(begin("Alex"));
    fed.until("vault", |s, _| {
        s.ceremony()
            .is_some_and(|c| c.phase == CeremonyPhase::Failed(CeremonyFailure::VaultUnavailable))
    });
    assert!(fed.store.ceremony().is_some_and(|c| c.ceremony_id.is_empty()), "no ceremony");
    fake.shutdown();

    // Wrong words spend the only attempt; nothing is created and every
    // secret copy is erased.
    let (fake, hub, mut fed, custody) = offering("wrong");
    let audit = hub.phrase_audit();
    challenged(&mut fed, true);
    let mut wrong = right_words();
    wrong[1] = SecretInput::new(demo_word(5));
    fed.source.submit(confirm(wrong));
    fed.until("mismatch", |s, _| {
        s.ceremony()
            .is_some_and(|c| c.phase == CeremonyPhase::Failed(CeremonyFailure::ConfirmationFailed))
    });
    assert_eq!(fed.store.accounts().count(), 0, "nothing was created");
    assert_eq!(custody.running(), 0);
    assert_eq!((audit.secrets(), audit.secrets_wiped()), (4, 4));
    fake.shutdown();

    // Out of order: a cancel with no ceremony is a failure of that call.
    let (fake, _hub, mut fed, _) = offering("noceremony");
    fed.source.submit(call(methods::CUSTODY_CANCEL, json!({})));
    fed.until("refused", |s, _| s.failures().iter().any(|f| f.method == methods::CUSTODY_CANCEL));
    assert_eq!(fed.store.failures()[0].code, FailureCode::Daemon("CustodySetupInvalid".to_owned()));
    fake.shutdown();
}

#[test]
fn the_passphrase_set_at_confirm_unlocks_after_a_lock_and_a_daemon_restart() {
    let custody = Arc::new(FakeCustody::new());
    let offered = Arc::new(AtomicBool::new(true));
    let (fake, path) = daemon("restart", &custody, &offered);
    let hub = SdkHub::start(config(&path)).expect("bridge");
    let mut fed = Fed::new(hub.daemon_source());
    fed.until("creation offered", |s, _| s.offers_born_v3_creation());
    challenged(&mut fed, true);
    fed.source.submit(confirm(right_words()));
    fed.until("created", |s, _| {
        s.ceremony().is_some_and(|c| matches!(c.phase, CeremonyPhase::Created(_)))
    });
    let Some(CeremonyPhase::Created(made)) = phase(&fed) else { panic!("created") };
    let id = AccountId::new(&made.local_account_id);
    fed.until("listed", |s, _| s.account(&id).is_some_and(|a| a.lock_state == LockState::Unlocked));

    // Lock it, then restart the daemon (a new bus over the same state).
    fed.source.submit(call(methods::ACCOUNT_LOCK, json!({ "account_id": id })));
    fed.until("locked", |s, _| s.account(&id).is_some_and(|a| a.lock_state == LockState::Locked));
    drop(fed);
    drop(hub);
    fake.shutdown();
    custody.restart();
    let (fake, path) = daemon("restart", &custody, &offered);
    let hub = SdkHub::start(config(&path)).expect("bridge");
    let mut fed = Fed::new(hub.daemon_source());
    fed.until("the account is listed locked, networking pending", |s, _| {
        s.account(&id).is_some_and(|a| {
            a.lock_state == LockState::Locked && a.networking == Networking::Pending
        })
    });
    let unlock = |passphrase: &str| Effect::SecretCall {
        method: methods::ACCOUNT_UNLOCK.to_owned(),
        body: json!({ "account_id": id }),
        secrets: vec![SecretInput::new(passphrase)],
    };
    // Another passphrase is refused; the one set at confirm opens it.
    fed.source.submit(unlock("not the passphrase"));
    fed.until("refused", |s, _| s.failures().iter().any(|f| f.method == methods::ACCOUNT_UNLOCK));
    assert_eq!(fed.store.account(&id).map(|a| a.lock_state), Some(LockState::Locked));
    fed.source.submit(unlock(PASSPHRASE));
    fed.until("unlocked", |s, _| {
        s.account(&id).is_some_and(|a| {
            a.lock_state == LockState::Unlocked && a.networking == Networking::Pending
        })
    });
    // The body of the unlock carries the passphrase under the contract's key.
    let sent =
        fake.seen().into_iter().rev().find(|s| s.method == "account.unlock").expect("unlock");
    assert_eq!(sent.body["passphrase"], PASSPHRASE);
    assert_eq!(sent.account.as_deref(), Some(id.as_str()));
    fake.shutdown();
}

#[test]
fn a_recovery_runs_on_its_own_connection_and_restores_a_locked_account() {
    let custody = Arc::new(FakeCustody::new());
    custody.add_locked_account("acct-r", "Alex", PASSPHRASE);
    let (fake, path) = daemon("recover", &custody, &Arc::new(AtomicBool::new(true)));
    let hub = SdkHub::start(config(&path)).expect("bridge");
    let audit = hub.phrase_audit();
    let mut fed = Fed::new(hub.daemon_source());
    let id = AccountId::new("acct-r");
    fed.until("listed locked", |s, _| {
        s.offers_born_v3_creation()
            && s.account(&id).is_some_and(|a| a.lock_state == LockState::Locked)
    });
    // Nothing starts a recovery but the user's request.
    assert!(fed.store.recovery().is_none());
    assert!(fake.seen().iter().all(|s| !s.method.starts_with("account.recover")));

    fed.source.submit(call(methods::RECOVER_BEGIN, json!({ "account_id": id })));
    fed.until("prepared", |s, _| s.recovery().is_some_and(|r| r.phase == RecoveryPhase::Prepared));
    let recovery = fed.store.recovery().expect("recovery").clone();
    assert_eq!(recovery.anchor, custody.anchor_of("acct-r").expect("anchor"));
    assert_eq!(recovery.account_id, id);

    let secrets = |phrase: String, passphrase: &str| Effect::SecretCall {
        method: methods::RECOVER_CONFIRM.to_owned(),
        body: json!({ "account_id": id, "confirmed_anchor": recovery.anchor }),
        secrets: vec![SecretInput::new(phrase), SecretInput::new(passphrase)],
    };
    fed.source.submit(secrets(fake_custody::phrase(), PASSPHRASE));
    fed.until("recovered", |s, _| {
        s.recovery().is_some_and(|r| matches!(r.phase, RecoveryPhase::Recovered(_)))
    });
    assert_eq!(fed.store.account(&id).map(|a| a.lock_state), Some(LockState::Unlocked));
    let sent =
        fake.seen().into_iter().find(|s| s.method == "account.recover.confirm").expect("sent");
    assert_eq!(sent.body["confirmed_anchor"], recovery.anchor);
    assert_eq!(
        custody.connections_of("account.recover.begin"),
        custody.connections_of("account.recover.confirm")
    );
    assert_eq!((audit.secrets(), audit.secrets_wiped()), (2, 2));
    assert_eq!(audit.issued(), 0, "recovery shows no phrase");
    let all = format!("{:?}", fed.envelopes);
    assert!(!all.contains(PASSPHRASE) && !all.contains(demo_word(3)), "no secret in events");
    fake.shutdown();
}

#[test]
fn a_recovery_with_a_wrong_phrase_or_an_unknown_account_changes_nothing() {
    let custody = Arc::new(FakeCustody::new());
    custody.add_locked_account("acct-r", "Alex", PASSPHRASE);
    let (fake, path) = daemon("recover-wrong", &custody, &Arc::new(AtomicBool::new(true)));
    let hub = SdkHub::start(config(&path)).expect("bridge");
    let audit = hub.phrase_audit();
    let mut fed = Fed::new(hub.daemon_source());
    let id = AccountId::new("acct-r");
    fed.until("listed", |s, _| s.offers_born_v3_creation() && s.account(&id).is_some());

    fed.source.submit(call(methods::RECOVER_BEGIN, json!({ "account_id": "acct-none" })));
    fed.until("refused", |s, _| {
        s.recovery().is_some_and(|r| r.phase == RecoveryPhase::Failed(CeremonyFailure::Invalid))
    });
    assert!(fed.store.recovery().is_some_and(|r| r.ceremony_id.is_empty()), "none was begun");

    fed.source.submit(call(methods::RECOVER_BEGIN, json!({ "account_id": id })));
    fed.until("prepared", |s, _| s.recovery().is_some_and(|r| r.is_active()));
    let anchor = fed.store.recovery().expect("recovery").anchor.clone();
    let wrong = (1..=24).map(|n| demo_word(n + 1)).collect::<Vec<_>>().join(" ");
    fed.source.submit(Effect::SecretCall {
        method: methods::RECOVER_CONFIRM.to_owned(),
        body: json!({ "account_id": id, "confirmed_anchor": anchor }),
        secrets: vec![SecretInput::new(wrong), SecretInput::new(PASSPHRASE)],
    });
    fed.until("mismatch", |s, _| {
        s.recovery()
            .is_some_and(|r| r.phase == RecoveryPhase::Failed(CeremonyFailure::ConfirmationFailed))
    });
    assert_eq!(fed.store.account(&id).map(|a| a.lock_state), Some(LockState::Locked));
    assert_eq!((audit.secrets(), audit.secrets_wiped()), (2, 2));
    assert_eq!(custody.running(), 0);

    // Cancel erases a prepared recovery.
    fed.source.submit(call(methods::RECOVER_BEGIN, json!({ "account_id": id })));
    fed.until("prepared again", |s, _| s.recovery().is_some_and(|r| r.is_active()));
    fed.source.submit(call(methods::RECOVER_CANCEL, json!({})));
    fed.until("cancelled", |s, _| {
        s.recovery().is_some_and(|r| r.phase == RecoveryPhase::Cancelled)
    });
    assert_eq!(custody.running(), 0);
    fake.shutdown();
}

#[test]
fn an_event_the_network_sends_never_starts_a_recovery() {
    let (fake, _hub, mut fed, custody) = offering("never");
    // The daemon pushes whatever it likes; the source starts nothing.
    fake.push("acct-main", "account.recovery.requested", json!({ "account_id": "acct-main" }));
    fed.settle(400);
    assert!(fed.store.recovery().is_none());
    assert_eq!(custody.connections_of("account.recover.begin").len(), 0);
    assert!(
        fed.envelopes.iter().all(|e| !matches!(e.event, DomainEvent::RecoveryUpdated(_))),
        "no recovery event without a user's request"
    );
    fake.shutdown();
}

#[test]
fn a_pending_account_is_read_again_until_it_is_live_and_the_banner_state_follows() {
    let custody = Arc::new(FakeCustody::new());
    let offered = Arc::new(AtomicBool::new(true));
    let (fake, path) = daemon("net-poll", &custody, &offered);
    let config = ramflux_desktop_sdk::SdkConfig {
        networking_poll_ms: [100, 400],
        // The ordinary poll is out of the way: only the backoff reads.
        poll_fast: Duration::from_secs(60),
        poll_slow: Duration::from_secs(60),
        ..common::config(&path)
    };
    let hub = SdkHub::start(config).expect("bridge");
    let mut daemon = Fed::new(hub.daemon_source());
    daemon.until("creation offered", |s, _| s.offers_born_v3_creation());
    // No default node was set: the account is made, but not connected.
    challenged(&mut daemon, true);
    daemon.source.submit(confirm(right_words()));
    daemon.until("created", |s, _| {
        s.ceremony().is_some_and(|c| matches!(c.phase, CeremonyPhase::Created(_)))
    });
    let Some(CeremonyPhase::Created(made)) = phase(&daemon) else { panic!("created") };
    let id = AccountId::new(&made.local_account_id);
    assert_eq!(made.networking, Networking::Pending);

    // The account's own scope reads its status, and reads it again.
    let mut account = Fed::new(hub.account_source(id.clone()));
    account.until("status says pending", |s, _| {
        s.account(&id).is_some_and(|a| a.networking == Networking::Pending)
    });
    let reads =
        |fake: &FakeBus| fake.seen().iter().filter(|s| s.method == "account.status").count();
    let before = reads(&fake);
    account.settle(700);
    assert!(reads(&fake) >= before + 2, "re-read with a backoff while pending");
    // The daemon connects to its node later: the very next read clears it.
    custody.go_live(id.as_str());
    account.until("live", |s, _| s.account(&id).is_some_and(|a| a.networking == Networking::Live));
    // Once live nothing keeps polling the status.
    account.settle(300);
    let settled = reads(&fake);
    account.settle(900);
    assert_eq!(reads(&fake), settled, "no reads once live");
    fake.shutdown();
}

#[test]
fn an_account_made_with_a_default_node_goes_live_inside_confirm() {
    let (fake, _hub, mut fed, custody) = offering("with-default");
    fed.source.submit(call(methods::NODE_SET_DEFAULT, json!({ "node_id": "n-harbor" })));
    let limit = std::time::Instant::now() + Duration::from_secs(10);
    while custody.default_node().is_none() {
        assert!(std::time::Instant::now() < limit, "the default was not set");
        fed.settle(20);
    }
    challenged(&mut fed, true);
    fed.source.submit(confirm(right_words()));
    fed.until("created", |s, _| {
        s.ceremony().is_some_and(|c| matches!(c.phase, CeremonyPhase::Created(_)))
    });
    let Some(CeremonyPhase::Created(made)) = phase(&fed) else { panic!("created") };
    assert_eq!(made.networking, Networking::Live);
    fake.shutdown();
}
