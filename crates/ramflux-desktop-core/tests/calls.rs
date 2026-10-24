// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Voice calls in the store (the call, its derived timeline record, the
//! state machine) and in policy (who may place, answer, decline and hang
//! up, where and when).

mod common;

use common::*;
use ramflux_desktop_core::*;

const ME: &str = "p-me";
const PEER: &str = "p-ana";

fn account() -> Account {
    serde_json::from_value(serde_json::json!({
        "id": "acct-main", "label": "Main", "lock_state": "Unlocked",
        "self_participant_id": ME,
    }))
    .expect("account")
}

fn direct(id: &str, peer: &str) -> Context {
    Context {
        kind: ContextKind::Direct,
        participants: vec![ParticipantId::new(ME), ParticipantId::new(peer)],
        ..context(id, false)
    }
}

fn feed() -> Feed {
    let mut feed = Feed::new();
    feed.ok(1, DomainEvent::AccountUpdated(account()));
    for (id, safety) in [
        (ME, SafetyState::Verified),
        (PEER, SafetyState::Unverified),
        ("p-cy", SafetyState::KeyChanged),
    ] {
        let mut p = participant(id, ParticipantKind::Human, id, true);
        p.safety = safety;
        feed.ok(1, DomainEvent::ParticipantUpserted(p));
    }
    feed.ok(1, DomainEvent::ContextUpserted(direct("c-ana", PEER)));
    feed.ok(1, DomainEvent::ContextUpserted(direct("c-cy", "p-cy")));
    feed.ok(1, DomainEvent::ContextUpserted(context("c-group", false)));
    feed
}

fn call(id: &str, direction: CallDirection, state: CallState, version: u64) -> Call {
    let caller = if direction == CallDirection::Outgoing { ME } else { PEER };
    Call {
        id: CallId::new(id),
        context_id: ContextId::new("c-ana"),
        peer: ParticipantId::new(PEER),
        caller: ParticipantId::new(caller),
        direction,
        state,
        version,
        started_at: Timestamp(1_000),
        answered_at: matches!(state, CallState::Active).then_some(Timestamp(5_000)),
        ended_at: None,
    }
}

fn upsert(call: Call) -> DomainEvent {
    DomainEvent::CallUpserted(call)
}

fn record(store: &Store, id: &str) -> Moment {
    store.moment(&Call::moment_id(&CallId::new(id))).expect("call record").clone()
}

#[test]
fn a_call_brings_its_record_which_follows_its_state() {
    let mut feed = feed();
    let changes =
        feed.ok(2, upsert(call("k1", CallDirection::Incoming, CallState::IncomingRinging, 1)));
    assert!(changes.calls.contains(&CallId::new("k1")));
    let moment = record(&feed.store, "k1");
    assert_eq!(moment.context_id, ContextId::new("c-ana"));
    assert_eq!(moment.origin, ParticipantId::new(PEER));
    assert_eq!(moment.at, Timestamp(1_000));
    assert_eq!(moment.lifecycle, Lifecycle::NeedsMe);
    assert!(matches!(&moment.kind, MomentKind::Call(p) if p.call_id == CallId::new("k1")));
    // A ringing call needs the user.
    let attention = selectors::attention(&feed.store);
    assert_eq!(attention.len(), 1);
    assert_eq!(attention[0].reason, AttentionReason::IncomingCall);
    assert_eq!(selectors::ringing_calls(&feed.store).len(), 1);
    assert!(selectors::current_call(&feed.store).is_none());

    feed.ok(3, upsert(call("k1", CallDirection::Incoming, CallState::Connecting, 2)));
    assert_eq!(record(&feed.store, "k1").lifecycle, Lifecycle::InProgress);
    assert!(selectors::attention(&feed.store).is_empty());
    assert_eq!(selectors::current_call(&feed.store).map(|c| c.id.as_str()), Some("k1"));
    feed.ok(4, upsert(call("k1", CallDirection::Incoming, CallState::Active, 3)));
    let ended = Call {
        state: CallState::Ended(CallEndReason::HungUp),
        version: 4,
        ended_at: Some(Timestamp(65_000)),
        ..call("k1", CallDirection::Incoming, CallState::Active, 3)
    };
    feed.ok(5, upsert(ended));
    assert_eq!(record(&feed.store, "k1").lifecycle, Lifecycle::Done);
    assert!(!selectors::call_under_way(&feed.store));
    let stored = feed.store.call(&CallId::new("k1")).unwrap();
    assert_eq!(stored.outcome(), Some(CallOutcome::Answered { duration_ms: 60_000 }));
    let text = export::timeline_text(&feed.store, &ContextId::new("c-ana"));
    assert!(text.iter().any(|line| line.contains("[call] incoming, 1:00")), "{text:?}");
    // The record is one moment, whatever happened to the call.
    assert_eq!(feed.store.context_moment_count(&ContextId::new("c-ana")), 1);
}

#[test]
fn stale_illegal_and_identity_changing_updates_are_refused() {
    let mut feed = feed();
    feed.ok(2, upsert(call("k1", CallDirection::Outgoing, CallState::OutgoingRinging, 5)));
    let stale = feed.push(3, upsert(call("k1", CallDirection::Outgoing, CallState::Connecting, 5)));
    assert!(matches!(stale, Err(ApplyError::StaleCallVersion { current: 5, found: 5, .. })));
    let skip = feed.push(3, upsert(call("k1", CallDirection::Outgoing, CallState::Active, 6)));
    assert!(matches!(skip, Err(ApplyError::IllegalCallTransition { .. })), "{skip:?}");
    let mut other = call("k1", CallDirection::Outgoing, CallState::Connecting, 6);
    other.peer = ParticipantId::new("p-cy");
    assert_eq!(
        feed.push(3, upsert(other)),
        Err(ApplyError::CallIdentityChanged(CallId::new("k1")))
    );
    let mut nowhere = call("k2", CallDirection::Outgoing, CallState::OutgoingRinging, 1);
    nowhere.context_id = ContextId::new("c-unknown");
    assert_eq!(
        feed.push(3, upsert(nowhere)),
        Err(ApplyError::UnknownContext(ContextId::new("c-unknown")))
    );
    // Nothing changed: still ringing at version 5.
    let stored = feed.store.call(&CallId::new("k1")).unwrap();
    assert_eq!((stored.state, stored.version), (CallState::OutgoingRinging, 5));
    // An ended call stays ended.
    let busy = Call {
        state: CallState::Ended(CallEndReason::Busy),
        ..call("k1", CallDirection::Outgoing, CallState::OutgoingRinging, 7)
    };
    feed.ok(4, upsert(busy));
    let again = feed.push(5, upsert(call("k1", CallDirection::Outgoing, CallState::Connecting, 8)));
    assert!(matches!(again, Err(ApplyError::IllegalCallTransition { .. })));
    // A lifecycle update from a source does not move a call's record.
    let _ = feed.push(6, set_lifecycle("call:k1", Lifecycle::Failed));
    assert_eq!(record(&feed.store, "k1").lifecycle, Lifecycle::Done);
}

#[test]
fn removing_the_conversation_removes_its_calls() {
    let mut feed = feed();
    feed.ok(2, upsert(call("k1", CallDirection::Incoming, CallState::IncomingRinging, 1)));
    let changes = feed.ok(3, DomainEvent::ContextRemoved { context_id: ContextId::new("c-ana") });
    assert!(changes.calls.contains(&CallId::new("k1")));
    assert!(feed.store.call(&CallId::new("k1")).is_none());
    assert!(selectors::ringing_calls(&feed.store).is_empty());
}

fn human(store: &Store, kind: IntentKind) -> Result<Vec<Effect>, IntentError> {
    handle_intent(store, Intent::human(kind), Timestamp(10))
}

fn start(context: &str) -> IntentKind {
    IntentKind::StartCall { context_id: ContextId::new(context) }
}

#[test]
fn a_call_is_placed_in_a_direct_conversation_to_its_peer() {
    let feed = feed();
    let effects = human(&feed.store, start("c-ana")).expect("placed");
    assert_eq!(effects[0].method(), Some(methods::CALL_INVITE));
    let body = effects[0].body().unwrap();
    assert_eq!(body["context_id"], "c-ana");
    assert_eq!(body["peer"], PEER);
    assert_eq!(body["proposed_by"], "Human");
    assert_eq!(
        check_start_call(&feed.store, &ContextId::new("c-ana")),
        Ok(ParticipantId::new(PEER))
    );

    assert_eq!(
        human(&feed.store, start("c-group")).unwrap_err(),
        IntentError::CallNeedsDirect(ContextId::new("c-group"))
    );
    assert_eq!(
        human(&feed.store, start("c-cy")).unwrap_err(),
        IntentError::SafetyUnresolved(ParticipantId::new("p-cy"))
    );
    assert_eq!(
        human(&feed.store, start("c-none")).unwrap_err(),
        IntentError::UnknownContext(ContextId::new("c-none"))
    );
}

#[test]
fn an_unreachable_peer_or_a_call_under_way_refuses_a_new_call() {
    let mut feed = feed();
    let mut ana = participant(PEER, ParticipantKind::Human, PEER, true);
    ana.reachable = Some(false);
    feed.ok(2, DomainEvent::ParticipantUpserted(ana.clone()));
    assert_eq!(
        human(&feed.store, start("c-ana")).unwrap_err(),
        IntentError::NotReachable(ParticipantId::new(PEER))
    );
    ana.reachable = Some(true);
    feed.ok(2, DomainEvent::ParticipantUpserted(ana));
    feed.ok(3, upsert(call("k1", CallDirection::Incoming, CallState::IncomingRinging, 1)));
    assert_eq!(human(&feed.store, start("c-ana")).unwrap_err(), IntentError::CallInProgress);
}

#[test]
fn agents_never_take_part_in_calls() {
    let mut feed = feed();
    feed.ok(
        1,
        DomainEvent::ParticipantUpserted(participant(
            "p-bot",
            ParticipantKind::Agent,
            "Bot",
            false,
        )),
    );
    feed.ok(2, upsert(call("k1", CallDirection::Incoming, CallState::IncomingRinging, 1)));
    let agent = |kind| {
        handle_intent(&feed.store, Intent::agent(ParticipantId::new("p-bot"), kind), Timestamp(10))
    };
    let k1 = || CallId::new("k1");
    assert!(matches!(agent(start("c-ana")), Err(IntentError::AgentForbidden(_))));
    assert!(matches!(
        agent(IntentKind::AnswerCall { call_id: k1() }),
        Err(IntentError::AgentForbidden(_))
    ));
    assert!(matches!(
        agent(IntentKind::RejectCall { call_id: k1() }),
        Err(IntentError::AgentForbidden(_))
    ));
    assert!(matches!(
        agent(IntentKind::HangUpCall { call_id: k1() }),
        Err(IntentError::AgentForbidden(_))
    ));
}

#[test]
fn answer_and_decline_apply_to_a_ringing_call_hang_up_to_one_under_way() {
    let mut feed = feed();
    let k = |id: &str| CallId::new(id);
    feed.ok(2, upsert(call("in", CallDirection::Incoming, CallState::IncomingRinging, 1)));
    let answer = human(&feed.store, IntentKind::AnswerCall { call_id: k("in") }).unwrap();
    assert_eq!(answer[0].method(), Some(methods::CALL_ANSWER));
    assert_eq!(answer[0].body().unwrap()["call_id"], "in");
    let reject = human(&feed.store, IntentKind::RejectCall { call_id: k("in") }).unwrap();
    assert_eq!(reject[0].method(), Some(methods::CALL_REJECT));
    let hang_up = human(&feed.store, IntentKind::HangUpCall { call_id: k("in") }).unwrap();
    assert_eq!(hang_up[0].method(), Some(methods::CALL_HANGUP));
    assert_eq!(
        human(&feed.store, IntentKind::AnswerCall { call_id: k("none") }).unwrap_err(),
        IntentError::UnknownCall(k("none"))
    );

    // An outgoing call cannot be answered from this side.
    feed.ok(
        3,
        upsert(Call {
            context_id: ContextId::new("c-cy"),
            peer: ParticipantId::new("p-cy"),
            ..call("out", CallDirection::Outgoing, CallState::OutgoingRinging, 1)
        }),
    );
    assert_eq!(
        human(&feed.store, IntentKind::AnswerCall { call_id: k("out") }).unwrap_err(),
        IntentError::CallState(k("out"))
    );
    // While this device is in (or placing) a call, a ringing one is not
    // answered over it; it can still be declined.
    assert_eq!(
        human(&feed.store, IntentKind::AnswerCall { call_id: k("in") }).unwrap_err(),
        IntentError::CallInProgress
    );
    assert!(human(&feed.store, IntentKind::RejectCall { call_id: k("in") }).is_ok());

    let ended = Call {
        state: CallState::Ended(CallEndReason::Timeout),
        ..call("in", CallDirection::Incoming, CallState::IncomingRinging, 2)
    };
    feed.ok(4, upsert(ended));
    for kind in [
        IntentKind::AnswerCall { call_id: k("in") },
        IntentKind::RejectCall { call_id: k("in") },
        IntentKind::HangUpCall { call_id: k("in") },
    ] {
        assert_eq!(human(&feed.store, kind).unwrap_err(), IntentError::CallState(k("in")));
    }
    assert_eq!(feed.store.call(&k("in")).and_then(Call::outcome), Some(CallOutcome::Missed));
}

#[test]
fn an_unsupported_backend_refuses_calls_before_anything_else() {
    let mut feed = feed();
    let mut unsupported = std::collections::BTreeMap::new();
    unsupported.insert(methods::CALL_INVITE.to_owned(), "backend.gap.calls".to_owned());
    feed.ok(2, DomainEvent::CapabilitiesUpdated(Capabilities { unsupported }));
    assert!(matches!(
        human(&feed.store, start("c-ana")),
        Err(IntentError::UnsupportedByBackend { method: "call.invite", .. })
    ));
}

#[test]
fn calls_and_their_events_round_trip_through_serde() {
    let event = upsert(Call {
        state: CallState::Ended(CallEndReason::AnsweredElsewhere),
        ..call("k1", CallDirection::Incoming, CallState::IncomingRinging, 2)
    });
    let json = serde_json::to_string(&event).unwrap();
    assert_eq!(serde_json::from_str::<DomainEvent>(&json).unwrap(), event);
    let ui = UiEvent::new(
        Scope::Workspace,
        UiEventKind::CallAnswered { call_id: CallId::new("k1"), accept: true },
    );
    let json = serde_json::to_string(&ui).unwrap();
    assert_eq!(serde_json::from_str::<UiEvent>(&json).unwrap(), ui);
}
