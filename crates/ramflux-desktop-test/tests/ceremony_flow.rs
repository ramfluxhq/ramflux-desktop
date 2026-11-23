// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The born-v3 creation ceremony end to end on the mock source: the
//! capability arrives late, each step is an intent, the phrase reaches the
//! caller only through the source's secret channel (never an event), the
//! three words are the one attempt, and the result is "networking pending"
//! with no account opened.

use ramflux_desktop_core::{
    AccountId, CEREMONY_TTL_MS, CeremonyFailure, CeremonyPhase, Intent, IntentKind, LockState,
    Networking, SecretInput, Source, Store,
};
use ramflux_desktop_test::mock_ceremony::{DEMO_POSITIONS, demo_word};
use ramflux_desktop_test::{ScenarioPlayer, load_fixture};

const PASSPHRASE: &str = "river stone lantern";

fn player() -> ScenarioPlayer {
    let mut p = ScenarioPlayer::new(&load_fixture("s13_born_v3").expect("s13"));
    p.advance(2_100);
    p
}

fn step(p: &mut ScenarioPlayer, kind: IntentKind) {
    p.dispatch(Intent::human(kind)).expect("allowed");
    p.advance(100);
}

fn phase(store: &Store) -> CeremonyPhase {
    store.ceremony().expect("ceremony").phase.clone()
}

fn pass() -> SecretInput {
    SecretInput::new(PASSPHRASE)
}

fn begin(p: &mut ScenarioPlayer) {
    step(p, IntentKind::BeginCeremony { label: "Alex".into() });
}

fn words(positions: [u8; 3]) -> Vec<SecretInput> {
    positions.iter().map(|p| SecretInput::new(demo_word(usize::from(*p)))).collect()
}

#[test]
fn the_capability_arrives_late_and_the_entry_is_refused_before_it() {
    let mut p = ScenarioPlayer::new(&load_fixture("s13_born_v3").expect("s13"));
    p.advance(1_000);
    assert!(!p.store().offers_born_v3_creation());
    let begin = Intent::human(IntentKind::BeginCeremony { label: "Alex".into() });
    assert!(p.dispatch(begin).is_err());
    p.advance(1_200);
    assert!(p.store().offers_born_v3_creation());
}

#[test]
fn a_ceremony_creates_an_account_that_is_networking_pending() {
    let mut p = player();
    begin(&mut p);
    assert_eq!(phase(p.store()), CeremonyPhase::Prepared);
    assert!(p.source_mut().take_recovery_phrase().is_none(), "nothing before show");

    step(&mut p, IntentKind::ShowRecoveryPhrase);
    assert_eq!(phase(p.store()), CeremonyPhase::PhraseReady);
    let phrase = p.source_mut().take_recovery_phrase().expect("the phrase");
    assert_eq!(phrase.word(3), Some(demo_word(3)));
    assert!(p.source_mut().take_recovery_phrase().is_none(), "handed out once");
    drop(phrase);

    step(&mut p, IntentKind::HideRecoveryPhrase);
    assert_eq!(phase(p.store()), CeremonyPhase::Challenged { positions: DEMO_POSITIONS });

    step(&mut p, IntentKind::ConfirmCeremony { words: words(DEMO_POSITIONS), passphrase: pass() });
    let CeremonyPhase::Created(account) = phase(p.store()) else {
        panic!("{:?}", phase(p.store()))
    };
    assert_eq!(account.display_name, "Alex");
    assert_eq!(account.networking, ramflux_desktop_core::Networking::Pending);
    // The ordinary account-create answer: listed, unlocked, networking pending.
    let listed = p.store().account(&AccountId::new(&account.local_account_id)).expect("listed");
    assert_eq!(listed.lock_state, LockState::Unlocked);
    assert_eq!(listed.networking, Networking::Pending);
    assert!(p.errors().is_empty(), "{:?}", p.errors());
}

#[test]
fn the_passphrase_set_at_confirm_unlocks_the_account_after_a_lock() {
    let mut p = player();
    begin(&mut p);
    step(&mut p, IntentKind::ShowRecoveryPhrase);
    drop(p.source_mut().take_recovery_phrase());
    step(&mut p, IntentKind::HideRecoveryPhrase);
    step(&mut p, IntentKind::ConfirmCeremony { words: words(DEMO_POSITIONS), passphrase: pass() });
    let id = AccountId::new("acct-v3-1");
    step(&mut p, IntentKind::LockAccount { account_id: id.clone() });
    assert_eq!(p.store().account(&id).expect("account").lock_state, LockState::Locked);
    // Not the words, not another passphrase.
    let unlock = |secret: &str| IntentKind::Unlock {
        account_id: id.clone(),
        secret: SecretInput::new(secret),
    };
    step(&mut p, unlock("abandon ability able"));
    assert_eq!(p.store().account(&id).expect("account").lock_state, LockState::Locked);
    assert!(!p.store().failures().is_empty(), "the refusal is reported");
    step(&mut p, unlock(PASSPHRASE));
    assert_eq!(p.store().account(&id).expect("account").lock_state, LockState::Unlocked);
}

#[test]
fn the_secrets_are_in_no_effect_body_and_no_event() {
    let mut p = player();
    begin(&mut p);
    step(&mut p, IntentKind::ShowRecoveryPhrase);
    drop(p.source_mut().take_recovery_phrase());
    step(&mut p, IntentKind::HideRecoveryPhrase);
    step(&mut p, IntentKind::ConfirmCeremony { words: words(DEMO_POSITIONS), passphrase: pass() });
    // Whole words only: a short word is a piece of an unrelated one.
    let leaks = |text: &str| {
        text.split(|c: char| !c.is_ascii_alphanumeric())
            .any(|token| (1..=24).any(|n| demo_word(n) == token))
    };
    for record in p.source().effects() {
        let body = record.body.to_string();
        assert!(!leaks(&body), "{}: {body}", record.label);
    }
    let store_text = format!("{:?}", p.store().ceremony());
    assert!(!leaks(&store_text), "{store_text}");
}

#[test]
fn a_wrong_word_is_the_end_of_the_only_attempt() {
    let mut p = player();
    begin(&mut p);
    step(&mut p, IntentKind::ShowRecoveryPhrase);
    drop(p.source_mut().take_recovery_phrase());
    step(&mut p, IntentKind::HideRecoveryPhrase);
    let mut typed = words(DEMO_POSITIONS);
    typed[2] = SecretInput::new(demo_word(5));
    step(&mut p, IntentKind::ConfirmCeremony { words: typed, passphrase: pass() });
    assert_eq!(phase(p.store()), CeremonyPhase::Failed(CeremonyFailure::ConfirmationFailed));
    let again = Intent::human(IntentKind::ConfirmCeremony {
        words: words(DEMO_POSITIONS),
        passphrase: pass(),
    });
    assert!(p.dispatch(again).is_err(), "nothing is left to confirm");
}

#[test]
fn a_ceremony_left_for_five_minutes_expires_on_its_next_step() {
    let mut p = player();
    begin(&mut p);
    p.advance(CEREMONY_TTL_MS.unsigned_abs());
    step(&mut p, IntentKind::ShowRecoveryPhrase);
    assert_eq!(phase(p.store()), CeremonyPhase::Failed(CeremonyFailure::Expired));
    assert!(p.source_mut().take_recovery_phrase().is_none(), "an expired phrase is erased");
}

#[test]
fn cancel_ends_the_ceremony_and_a_new_one_may_begin() {
    let mut p = player();
    begin(&mut p);
    step(&mut p, IntentKind::ShowRecoveryPhrase);
    step(&mut p, IntentKind::CancelCeremony);
    assert_eq!(phase(p.store()), CeremonyPhase::Cancelled);
    assert!(p.source_mut().take_recovery_phrase().is_none(), "cancel erases an untaken phrase");
    begin(&mut p);
    assert_eq!(phase(p.store()), CeremonyPhase::Prepared);
    assert!(p.errors().is_empty(), "{:?}", p.errors());
}

#[test]
fn a_connected_account_is_reported_as_live() {
    let mut p = player();
    p.source_mut().ceremony_mut().set_networking(ramflux_desktop_core::Networking::Live);
    begin(&mut p);
    step(&mut p, IntentKind::ShowRecoveryPhrase);
    drop(p.source_mut().take_recovery_phrase());
    step(&mut p, IntentKind::HideRecoveryPhrase);
    step(&mut p, IntentKind::ConfirmCeremony { words: words(DEMO_POSITIONS), passphrase: pass() });
    let CeremonyPhase::Created(account) = phase(p.store()) else { panic!("account") };
    assert_eq!(account.networking, ramflux_desktop_core::Networking::Live);
}
