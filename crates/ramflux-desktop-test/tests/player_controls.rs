// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Timeline controls of the scenario player and mock source behavior.

use ramflux_desktop_core::*;
use ramflux_desktop_test::{EffectMatcher, MockSource, ScenarioPlayer, load_fixture};

fn approve(grant: &str, confirmed: bool) -> Intent {
    Intent::human(IntentKind::Approve { grant_id: GrantId::new(grant), confirmed })
}

#[test]
fn pause_freezes_advance_but_step_still_works() {
    let scenario = load_fixture("s02_daily").expect("fixture");
    let mut p = ScenarioPlayer::new(&scenario);
    p.pause();
    assert!(p.is_paused());
    assert_eq!(p.advance(10_000), 0);
    assert_eq!(p.store().last_seq(), 0);

    assert!(p.step());
    assert_eq!(p.store().last_seq(), 1);
    p.resume();
    assert!(p.advance(10) > 0);
}

#[test]
fn step_moves_time_to_the_next_event() {
    let scenario = load_fixture("s04_safety").expect("fixture");
    let mut p = ScenarioPlayer::new(&scenario);
    p.advance(100);
    assert!(p.step());
    assert_eq!(p.now_ms(), 500);
    let total = p.run_to_end();
    assert!(total > 0);
    assert!(!p.step());
    assert_eq!(p.now_ms(), 500);
}

#[test]
fn seek_backwards_rebuilds_the_store() {
    let scenario = load_fixture("s04_safety").expect("fixture");
    let mut p = ScenarioPlayer::new(&scenario);
    p.advance(6000);
    let verify = IntentKind::VerifySafetyNumber { participant_id: ParticipantId::new("p-mira") };
    p.dispatch(Intent::human(verify)).expect("verify");
    p.advance(200);
    let full = p.store().last_seq();
    assert_eq!(
        p.store().moment(&MomentId::new("m-key")).expect("m-key").lifecycle,
        Lifecycle::Done
    );

    p.seek(1000);

    let key = p.store().moment(&MomentId::new("m-key")).expect("m-key");
    assert_eq!(key.lifecycle, Lifecycle::NeedsMe);
    assert!(p.store().last_seq() < full);
    p.seek(6200);
    assert_eq!(p.store().last_seq(), full);
    assert!(p.errors().is_empty());
}

#[test]
fn seek_replays_triggered_responses_deterministically() {
    let scenario = load_fixture("s03_intervention").expect("fixture");
    let mut p = ScenarioPlayer::new(&scenario);
    p.advance(1000);
    p.dispatch(approve("g-rotate", true)).expect("approve");
    p.advance(2000);
    let before = format!("{:?}", p.store());

    p.seek(500);
    assert!(p.store().grant(&GrantId::new("g-rotate")).is_none());
    p.seek(3000);

    assert_eq!(format!("{:?}", p.store()), before);
    assert!(p.errors().is_empty());
}

#[test]
fn mock_source_assigns_increasing_seq_and_stamps_moment_time() {
    let scenario = load_fixture("s02_daily").expect("fixture");
    let mut source = MockSource::new(&scenario);
    let envelopes = source.drain(scenario.epoch.saturating_add_ms(10_000));

    let seqs: Vec<u64> = envelopes.iter().map(|e| e.seq).collect();
    let expected: Vec<u64> = (1..=envelopes.len() as u64).collect();
    assert_eq!(seqs, expected);
    assert!(envelopes.windows(2).all(|w| w[0].at <= w[1].at));
    for env in &envelopes {
        assert_eq!(env.schema, EVENT_SCHEMA);
        if let DomainEvent::MomentAdded(moment) = &env.event {
            assert_eq!(moment.at, env.at);
        }
    }
    assert!(source.drain(scenario.epoch.saturating_add_ms(20_000)).is_empty());
}

#[test]
fn unmatched_effects_are_recorded_without_events() {
    let scenario = load_fixture("s04_safety").expect("fixture");
    let mut source = MockSource::new(&scenario);
    source.submit(Effect::Call { method: "context.pin".into(), body: JsonValue::Null });
    assert_eq!(source.effects().len(), 1);
    assert_eq!(source.effects()[0].rule, None);
}

#[test]
fn effect_matcher_checks_method_and_fields() {
    let matcher = EffectMatcher::Call {
        method: methods::GRANT_APPROVE.to_owned(),
        fields: [("grant_id".to_owned(), JsonValue::from("g-1"))].into_iter().collect(),
    };
    let call = |grant: &str| Effect::Call {
        method: methods::GRANT_APPROVE.to_owned(),
        body: JsonValue::Object(
            [("grant_id".to_owned(), JsonValue::from(grant))].into_iter().collect(),
        ),
    };
    assert!(matcher.matches(&call("g-1")));
    assert!(!matcher.matches(&call("g-2")));
    let unlock =
        Effect::Unlock { account_id: AccountId::new("a-1"), secret: SecretInput::new("x") };
    assert!(!matcher.matches(&unlock));
    assert!(EffectMatcher::Unlock { account_id: None }.matches(&unlock));
}

#[test]
fn replay_is_deterministic_across_players() {
    let scenario = load_fixture("s06_in_progress").expect("fixture");
    let run = || {
        let mut p = ScenarioPlayer::new(&scenario);
        p.advance(200);
        let start = IntentKind::StartTask {
            context_id: ContextId::new("c-trip"),
            request: "Book a train".into(),
        };
        p.dispatch(Intent::human(start)).expect("start");
        p.advance(3000);
        p.dispatch(approve("g-hold", false)).expect("approve");
        p.advance(3000);
        format!("{:?}", p.store())
    };
    assert_eq!(run(), run());
}
