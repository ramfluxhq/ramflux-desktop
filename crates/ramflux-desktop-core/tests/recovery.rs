// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The recovery ceremony of a born-v3 account in the store (the phase
//! machine, the fixed id and expiry, no words anywhere) and in policy
//! (offered only when the daemon says so, human only, a person's account,
//! each step only in its phase, the fingerprint the daemon named, 24 words).

mod common;

use common::*;
use ramflux_desktop_core::*;

const ANCHOR: &str = "anchor-abc";

fn recovery(phase: RecoveryPhase) -> RecoveryCeremony {
    RecoveryCeremony {
        phase,
        account_id: AccountId::new("a-1"),
        anchor: ANCHOR.to_owned(),
        ceremony_id: "recovery-1".to_owned(),
        expires_at: Timestamp(300_000),
    }
}

fn update(phase: RecoveryPhase) -> DomainEvent {
    DomainEvent::RecoveryUpdated(recovery(phase))
}

fn listed(kind: AccountKind) -> DomainEvent {
    DomainEvent::AccountUpdated(Account {
        id: AccountId::new("a-1"),
        label: "Alex".to_owned(),
        kind,
        node_id: None,
        device_id: None,
        lock_state: LockState::Locked,
        self_participant_id: None,
        secure_sessions: SecureSessions::Ready,
        networking: Networking::Live,
    })
}

fn offered() -> Feed {
    let mut feed = Feed::new();
    feed.ok(
        1,
        DomainEvent::CapabilitiesUpdated(Capabilities {
            born_v3_creation: true,
            ..Capabilities::default()
        }),
    );
    feed.ok(2, listed(AccountKind::HumanInteractive));
    feed
}

fn decide(store: &Store, kind: IntentKind) -> Result<Vec<Effect>, IntentError> {
    handle_intent(store, Intent::human(kind), Timestamp(10))
}

fn begin() -> IntentKind {
    IntentKind::BeginRecovery { account_id: AccountId::new("a-1") }
}

fn phrase_of(count: usize) -> SecretInput {
    SecretInput::new(vec!["abandon"; count].join(" "))
}

fn confirm(anchor: &str, phrase: SecretInput) -> IntentKind {
    IntentKind::ConfirmRecovery {
        anchor: anchor.to_owned(),
        phrase,
        passphrase: SecretInput::new("river stone lantern"),
    }
}

fn account() -> CeremonyAccount {
    CeremonyAccount {
        local_account_id: "a-1".to_owned(),
        display_name: "Alex".to_owned(),
        networking: Networking::Pending,
    }
}

#[test]
fn begin_needs_an_offer_a_known_person_account_and_no_recovery_running() {
    let mut feed = Feed::new();
    feed.ok(2, listed(AccountKind::HumanInteractive));
    assert_eq!(decide(&feed.store, begin()).err(), Some(IntentError::CeremonyNotOffered));

    let mut feed = offered();
    let unknown = IntentKind::BeginRecovery { account_id: AccountId::new("nope") };
    assert!(matches!(decide(&feed.store, unknown), Err(IntentError::UnknownAccount(_))));
    let effects = decide(&feed.store, begin()).expect("begin");
    let [Effect::Call { method, body }] = effects.as_slice() else { panic!("one call") };
    assert_eq!(method, methods::RECOVER_BEGIN);
    assert_eq!(body["account_id"], "a-1");
    feed.ok(3, update(RecoveryPhase::Prepared));
    assert_eq!(decide(&feed.store, begin()).err(), Some(IntentError::RecoveryPhase));
    feed.ok(4, update(RecoveryPhase::Cancelled));
    assert!(decide(&feed.store, begin()).is_ok(), "an ended recovery makes room");

    let mut ai = Feed::new();
    ai.ok(
        1,
        DomainEvent::CapabilitiesUpdated(Capabilities {
            born_v3_creation: true,
            ..Capabilities::default()
        }),
    );
    ai.ok(2, listed(AccountKind::HeadlessAi));
    assert_eq!(decide(&ai.store, begin()).err(), Some(IntentError::RecoveryNotForKind));
}

#[test]
fn an_agent_cannot_start_or_answer_a_recovery() {
    let feed = offered();
    let agent = ParticipantId::new("p-bot");
    let attempt = handle_intent(&feed.store, Intent::agent(agent, begin()), Timestamp(10));
    assert!(attempt.is_err(), "recovery is the person's act");
}

#[test]
fn the_recovery_walks_through_the_store_and_changes_say_so() {
    let mut feed = offered();
    assert!(feed.store.recovery().is_none());
    let before = feed.store.recovery_revision();
    let changes = feed.ok(3, update(RecoveryPhase::Prepared));
    assert!(changes.ceremony && !changes.is_empty());
    feed.ok(4, update(RecoveryPhase::Recovered(account())));
    let last = feed.store.recovery().expect("recovery");
    assert!(!last.is_active());
    assert_eq!(last.phase, RecoveryPhase::Recovered(account()));
    assert_eq!(feed.store.recovery_revision(), before + 2);
}

#[test]
fn an_update_without_a_begin_or_after_the_end_is_refused_and_changes_nothing() {
    let mut feed = offered();
    assert!(feed.push(3, update(RecoveryPhase::Recovered(account()))).is_err(), "no begin yet");
    assert!(feed.push(3, update(RecoveryPhase::Cancelled)).is_err());
    feed.ok(4, update(RecoveryPhase::Prepared));
    assert!(matches!(
        feed.push(5, update(RecoveryPhase::Prepared)),
        Err(ApplyError::IllegalRecoveryTransition { .. })
    ));
    feed.ok(6, update(RecoveryPhase::Failed(CeremonyFailure::Expired)));
    // A late answer to the ended recovery is not news.
    assert!(feed.push(7, update(RecoveryPhase::Failed(CeremonyFailure::Invalid))).is_err());
    assert!(feed.push(8, update(RecoveryPhase::Recovered(account()))).is_err());
    assert_eq!(
        feed.store.recovery().expect("recovery").phase,
        RecoveryPhase::Failed(CeremonyFailure::Expired)
    );
}

#[test]
fn the_account_id_and_expiry_of_a_running_recovery_cannot_change() {
    let mut feed = offered();
    feed.ok(3, update(RecoveryPhase::Prepared));
    let mut other = recovery(RecoveryPhase::Recovered(account()));
    other.ceremony_id = "recovery-2".to_owned();
    assert_eq!(
        feed.push(4, DomainEvent::RecoveryUpdated(other)).err(),
        Some(ApplyError::RecoveryChanged)
    );
    let mut elsewhere = recovery(RecoveryPhase::Recovered(account()));
    elsewhere.account_id = AccountId::new("a-2");
    assert_eq!(
        feed.push(5, DomainEvent::RecoveryUpdated(elsewhere)).err(),
        Some(ApplyError::RecoveryChanged)
    );
}

#[test]
fn a_refused_begin_is_a_failure_with_no_id() {
    let mut feed = offered();
    let refused = RecoveryCeremony {
        phase: RecoveryPhase::Failed(CeremonyFailure::Invalid),
        account_id: AccountId::new("a-1"),
        anchor: String::new(),
        ceremony_id: String::new(),
        expires_at: Timestamp(0),
    };
    feed.ok(3, DomainEvent::RecoveryUpdated(refused));
    assert!(!feed.store.recovery().expect("recovery").is_active());
    feed.ok(4, update(RecoveryPhase::Prepared));
}

#[test]
fn confirm_needs_the_named_fingerprint_24_words_and_a_passphrase() {
    let mut feed = offered();
    assert_eq!(
        decide(&feed.store, confirm(ANCHOR, phrase_of(24))).err(),
        Some(IntentError::RecoveryPhase),
        "nothing was begun"
    );
    feed.ok(3, update(RecoveryPhase::Prepared));
    assert_eq!(
        decide(&feed.store, confirm("other", phrase_of(24))).err(),
        Some(IntentError::RecoveryAnchor)
    );
    assert_eq!(
        decide(&feed.store, confirm(ANCHOR, phrase_of(23))).err(),
        Some(IntentError::RecoveryWords)
    );
    let bare = IntentKind::ConfirmRecovery {
        anchor: ANCHOR.to_owned(),
        phrase: phrase_of(24),
        passphrase: SecretInput::new(""),
    };
    assert!(matches!(decide(&feed.store, bare), Err(IntentError::Empty(_))));
    let effects = decide(&feed.store, confirm(ANCHOR, phrase_of(24))).expect("confirm");
    let [Effect::SecretCall { method, body, secrets }] = effects.as_slice() else {
        panic!("one secret call")
    };
    assert_eq!(method, methods::RECOVER_CONFIRM);
    assert_eq!(secrets.len(), 2, "the phrase, then the passphrase");
    assert_eq!(body["confirmed_anchor"], ANCHOR);
    assert_eq!(body["account_id"], "a-1");
    assert!(!body.to_string().contains("abandon"), "the words travel beside the body");
}

#[test]
fn cancel_needs_a_running_recovery() {
    let mut feed = offered();
    assert_eq!(
        decide(&feed.store, IntentKind::CancelRecovery).err(),
        Some(IntentError::RecoveryPhase)
    );
    feed.ok(3, update(RecoveryPhase::Prepared));
    let effects = decide(&feed.store, IntentKind::CancelRecovery).expect("cancel");
    assert_eq!(effects[0].method(), Some(methods::RECOVER_CANCEL));
    feed.ok(4, update(RecoveryPhase::Cancelled));
    assert!(decide(&feed.store, IntentKind::CancelRecovery).is_err());
}

#[test]
fn the_recovery_record_holds_no_words() {
    let text = serde_json::to_string(&update(RecoveryPhase::Prepared)).expect("json");
    assert!(!text.contains("abandon") && !text.contains("phrase"), "{text}");
}
