// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The creation ceremony of a born-v3 account in the store (the phase
//! machine, the fixed ceremony_id and expiry, no phrase anywhere) and in policy
//! (offered only when the daemon says so, human only, each step only in its
//! phase, exactly three words).

mod common;

use common::*;
use ramflux_desktop_core::*;

const CEREMONY_ID: &str = "ceremony-1";

fn ceremony(phase: CeremonyPhase) -> Ceremony {
    Ceremony {
        phase,
        ceremony_id: CEREMONY_ID.to_owned(),
        label: "Alex".to_owned(),
        expires_at: Timestamp(300_000),
    }
}

fn update(phase: CeremonyPhase) -> DomainEvent {
    DomainEvent::CeremonyUpdated(ceremony(phase))
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
    feed
}

fn decide(store: &Store, kind: IntentKind) -> Result<Vec<Effect>, IntentError> {
    handle_intent(store, Intent::human(kind), Timestamp(10))
}

fn words(count: usize) -> Vec<SecretInput> {
    (0..count).map(|_| SecretInput::new("abandon")).collect()
}

fn challenged() -> CeremonyPhase {
    CeremonyPhase::Challenged { positions: [3, 11, 20] }
}

fn account() -> CeremonyAccount {
    CeremonyAccount {
        local_account_id: "a-1".to_owned(),
        display_name: "Alex".to_owned(),
        networking: Networking::Pending,
    }
}

#[test]
fn the_daemon_offers_the_ceremony_only_when_it_says_so() {
    let mut feed = Feed::new();
    assert!(!feed.store.offers_born_v3_creation(), "off by default");
    // Declaring other methods unsupported does not switch it on.
    feed.ok(1, DomainEvent::CapabilitiesUpdated(Capabilities::default()));
    assert!(!feed.store.offers_born_v3_creation());
    let begin = IntentKind::BeginCeremony { label: "Alex".into() };
    assert_eq!(decide(&feed.store, begin).err(), Some(IntentError::CeremonyNotOffered));
    feed.ok(
        2,
        DomainEvent::CapabilitiesUpdated(Capabilities {
            born_v3_creation: true,
            ..Capabilities::default()
        }),
    );
    assert!(feed.store.offers_born_v3_creation());
}

#[test]
fn the_ceremony_walks_through_the_store_and_changes_say_so() {
    let mut feed = offered();
    assert!(feed.store.ceremony().is_none());
    let changes = feed.ok(2, update(CeremonyPhase::Prepared));
    assert!(changes.ceremony && !changes.is_empty());
    feed.ok(3, update(CeremonyPhase::PhraseReady));
    feed.ok(4, update(challenged()));
    feed.ok(5, update(CeremonyPhase::Created(account())));
    let last = feed.store.ceremony().expect("ceremony");
    assert!(!last.is_active());
    assert_eq!(last.phase, CeremonyPhase::Created(account()));
}

#[test]
fn an_update_that_skips_or_goes_back_is_refused_and_changes_nothing() {
    let mut feed = offered();
    assert!(feed.push(2, update(CeremonyPhase::PhraseReady)).is_err(), "no begin yet");
    feed.ok(3, update(CeremonyPhase::Prepared));
    assert!(matches!(
        feed.push(4, update(challenged())),
        Err(ApplyError::IllegalCeremonyTransition { .. })
    ));
    feed.ok(5, update(CeremonyPhase::PhraseReady));
    assert!(feed.push(6, update(CeremonyPhase::Prepared)).is_err(), "no going back");
    assert_eq!(feed.store.ceremony().expect("ceremony").phase, CeremonyPhase::PhraseReady);
}

#[test]
fn the_anchor_and_expiry_of_a_running_ceremony_cannot_change() {
    let mut feed = offered();
    feed.ok(2, update(CeremonyPhase::Prepared));
    let mut other = ceremony(CeremonyPhase::PhraseReady);
    other.ceremony_id = "ceremony-2".to_owned();
    assert_eq!(
        feed.push(3, DomainEvent::CeremonyUpdated(other)).err(),
        Some(ApplyError::CeremonyChanged)
    );
    let mut later = ceremony(CeremonyPhase::PhraseReady);
    later.expires_at = Timestamp(900_000);
    assert_eq!(
        feed.push(4, DomainEvent::CeremonyUpdated(later)).err(),
        Some(ApplyError::CeremonyChanged)
    );
}

#[test]
fn an_ended_ceremony_keeps_its_record_until_a_new_one_begins() {
    let mut feed = offered();
    feed.ok(2, update(CeremonyPhase::Prepared));
    feed.ok(3, update(CeremonyPhase::Failed(CeremonyFailure::Expired)));
    // A late failure of the ended ceremony is not news.
    assert!(feed.push(4, update(CeremonyPhase::Failed(CeremonyFailure::Invalid))).is_err());
    assert!(feed.push(5, update(CeremonyPhase::Cancelled)).is_err());
    assert_eq!(
        feed.store.ceremony().expect("ceremony").phase,
        CeremonyPhase::Failed(CeremonyFailure::Expired)
    );
    let mut again = ceremony(CeremonyPhase::Prepared);
    again.ceremony_id = "ceremony-2".to_owned();
    feed.ok(6, DomainEvent::CeremonyUpdated(again));
    assert_eq!(feed.store.ceremony().expect("ceremony").ceremony_id, "ceremony-2");
}

#[test]
fn a_refused_begin_is_a_failure_with_no_anchor() {
    let mut feed = offered();
    let refused = Ceremony {
        phase: CeremonyPhase::Failed(CeremonyFailure::Disabled),
        ceremony_id: String::new(),
        label: "Alex".to_owned(),
        expires_at: Timestamp(0),
    };
    feed.ok(2, DomainEvent::CeremonyUpdated(refused.clone()));
    assert!(!feed.store.ceremony().expect("ceremony").is_active());
    // The user may try again after a refusal.
    feed.ok(3, update(CeremonyPhase::Prepared));
}

#[test]
fn the_ceremony_and_its_events_hold_no_phrase() {
    // The store and the events serialize and debug-print; neither has a
    // place for the words, and the phrase type refuses both.
    let event = update(challenged());
    let text = serde_json::to_string(&event).expect("json");
    assert!(!text.contains("abandon") && !text.contains("words"), "{text}");
    let phrase =
        RecoveryPhrase::parse(zeroize::Zeroizing::new(vec!["zoo"; 24].join(" "))).expect("phrase");
    assert_eq!(format!("{phrase:?}"), "<redacted>");
}

#[test]
fn begin_needs_an_offer_a_name_and_no_ceremony_running() {
    let mut feed = offered();
    let begin = |label: &str| IntentKind::BeginCeremony { label: label.to_owned() };
    assert_eq!(decide(&feed.store, begin("  ")).err(), Some(IntentError::Empty("account name")));
    let effects = decide(&feed.store, begin(" Alex ")).expect("begin");
    let [Effect::Call { method, body }] = effects.as_slice() else { panic!("one call") };
    assert_eq!(method, methods::CUSTODY_BEGIN);
    assert_eq!(body["display_name"], "Alex");
    assert_eq!(body["principal_kind"], "human");
    feed.ok(2, update(CeremonyPhase::Prepared));
    assert_eq!(decide(&feed.store, begin("Alex")).err(), Some(IntentError::CeremonyPhase));
    feed.ok(3, update(CeremonyPhase::Cancelled));
    assert!(decide(&feed.store, begin("Alex")).is_ok(), "an ended ceremony makes room");
}

#[test]
fn each_step_is_allowed_only_in_its_phase() {
    let mut feed = offered();
    let show = || IntentKind::ShowRecoveryPhrase;
    let hide = || IntentKind::HideRecoveryPhrase;
    let confirm = || IntentKind::ConfirmCeremony {
        words: words(3),
        passphrase: SecretInput::new("river stone lantern"),
    };
    for kind in [show(), hide(), confirm(), IntentKind::CancelCeremony] {
        assert_eq!(decide(&feed.store, kind).err(), Some(IntentError::CeremonyPhase));
    }
    feed.ok(2, update(CeremonyPhase::Prepared));
    assert!(decide(&feed.store, show()).is_ok());
    assert!(decide(&feed.store, hide()).is_err() && decide(&feed.store, confirm()).is_err());
    feed.ok(3, update(CeremonyPhase::PhraseReady));
    assert!(decide(&feed.store, show()).is_err(), "the phrase is shown once");
    let effects = decide(&feed.store, hide()).expect("hide");
    assert_eq!(effects[0].method(), Some(methods::CUSTODY_HIDDEN_CHALLENGE));
    assert!(decide(&feed.store, confirm()).is_err(), "no words are asked before hide");
    feed.ok(4, update(challenged()));
    assert!(decide(&feed.store, hide()).is_err() && decide(&feed.store, show()).is_err());
    let effects = decide(&feed.store, confirm()).expect("confirm");
    let [Effect::SecretCall { method, secrets, body }] = effects.as_slice() else {
        panic!("one secret call")
    };
    assert_eq!(method, methods::CUSTODY_CONFIRM);
    assert_eq!(secrets.len(), 4, "three words, then the passphrase");
    assert!(!body.to_string().contains("abandon"), "the words travel beside the body");
    let cancel = decide(&feed.store, IntentKind::CancelCeremony).expect("cancel");
    assert_eq!(cancel[0].method(), Some(methods::CUSTODY_CANCEL));
    feed.ok(5, update(CeremonyPhase::Created(account())));
    assert!(decide(&feed.store, IntentKind::CancelCeremony).is_err(), "nothing left to cancel");
}

#[test]
fn the_one_attempt_is_not_spent_on_the_wrong_number_of_words() {
    let mut feed = offered();
    feed.ok(2, update(CeremonyPhase::Prepared));
    feed.ok(3, update(CeremonyPhase::PhraseReady));
    feed.ok(4, update(challenged()));
    for count in [0, 2, 4] {
        let kind = IntentKind::ConfirmCeremony {
            words: words(count),
            passphrase: SecretInput::new("river stone lantern"),
        };
        assert_eq!(decide(&feed.store, kind).err(), Some(IntentError::CeremonyWords));
    }
    let blank = vec![SecretInput::new("abandon"), SecretInput::new(""), SecretInput::new("zoo")];
    let kind = IntentKind::ConfirmCeremony {
        words: blank,
        passphrase: SecretInput::new("river stone lantern"),
    };
    assert_eq!(decide(&feed.store, kind).err(), Some(IntentError::CeremonyWords));
}

#[test]
fn an_agent_never_creates_an_account_with_a_phrase() {
    let mut feed = offered();
    feed.ok(
        2,
        DomainEvent::ParticipantUpserted(Participant {
            id: ParticipantId::new("p-orbit"),
            kind: ParticipantKind::Agent,
            display: "Orbit".to_owned(),
            ..serde_json::from_value(serde_json::json!({
                "id": "p-orbit", "kind": "Agent", "display": "Orbit",
            }))
            .expect("participant")
        }),
    );
    let intent = Intent::agent(
        ParticipantId::new("p-orbit"),
        IntentKind::BeginCeremony { label: "Alex".into() },
    );
    assert!(matches!(
        handle_intent(&feed.store, intent, Timestamp(10)),
        Err(IntentError::AgentForbidden(_))
    ));
}

#[test]
fn confirming_needs_a_passphrase() {
    let mut feed = offered();
    feed.ok(2, update(CeremonyPhase::Prepared));
    feed.ok(3, update(CeremonyPhase::PhraseReady));
    feed.ok(4, update(challenged()));
    let kind = IntentKind::ConfirmCeremony { words: words(3), passphrase: SecretInput::new("") };
    assert_eq!(decide(&feed.store, kind).err(), Some(IntentError::Empty("account passphrase")));
}
