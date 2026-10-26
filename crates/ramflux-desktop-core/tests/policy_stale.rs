// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A stale verification (the contact's identity authority changed since the
//! user verified it) refuses nothing: a new conversation, a new group and a
//! call are allowed, and the UI asks the user to acknowledge it. A key
//! change or a fork still refuses all three until verified again.

mod common;

use common::*;
use ramflux_desktop_core::*;

fn person(id: &str, safety: SafetyState) -> DomainEvent {
    let mut p = participant(id, ParticipantKind::Human, &format!("Name {id}"), true);
    p.link = ContactLink::Accepted;
    p.safety = safety;
    DomainEvent::ParticipantUpserted(p)
}

fn direct(id: &str, peer: &str) -> DomainEvent {
    let mut context = context(id, false);
    context.kind = ContextKind::Direct;
    context.participants = vec![ParticipantId::new("p-me"), ParticipantId::new(peer)];
    DomainEvent::ContextUpserted(context)
}

fn store() -> Store {
    let mut feed = Feed::new();
    feed.ok(
        1,
        DomainEvent::AccountUpdated(
            serde_json::from_value(serde_json::json!({
                "id": "acct-main", "label": "Main", "lock_state": "Unlocked",
                "self_participant_id": "p-me",
            }))
            .expect("account"),
        ),
    );
    feed.ok(1, person("p-me", SafetyState::Verified));
    feed.ok(1, person("p-stale", SafetyState::VerificationStale));
    feed.ok(1, person("p-changed", SafetyState::KeyChanged));
    feed.ok(1, person("p-forked", SafetyState::Forked));
    for (context, peer) in
        [("c-stale", "p-stale"), ("c-changed", "p-changed"), ("c-forked", "p-forked")]
    {
        feed.ok(1, direct(context, peer));
    }
    feed.store
}

fn human(store: &Store, kind: IntentKind) -> Result<Vec<Effect>, IntentError> {
    handle_intent(store, Intent::human(kind), Timestamp(10))
}

fn start(participant: &str) -> IntentKind {
    IntentKind::StartConversation {
        context_id: ContextId::new("c-new"),
        participant_id: ParticipantId::new(participant),
        text: "Hello".to_owned(),
    }
}

fn group(members: &[&str]) -> IntentKind {
    IntentKind::CreateGroup {
        context_id: ContextId::new("c-group"),
        title: "Trip".to_owned(),
        members: members.iter().map(|m| ParticipantId::new(*m)).collect(),
    }
}

#[test]
fn a_stale_verification_allows_a_new_conversation() {
    let store = store();
    assert_eq!(check_start_conversation(&store, &ParticipantId::new("p-stale")), Ok(()));
    let effects = human(&store, start("p-stale")).expect("allowed");
    assert!(
        matches!(effects.as_slice(), [Effect::Call { method, .. }] if method == methods::CONVERSATION_START)
    );
}

#[test]
fn a_stale_verification_allows_a_new_group() {
    let store = store();
    let effects = human(&store, group(&["p-stale"])).expect("allowed");
    assert!(!effects.is_empty());
}

#[test]
fn a_stale_verification_allows_a_call() {
    let store = store();
    assert_eq!(
        check_start_call(&store, &ContextId::new("c-stale")),
        Ok(ParticipantId::new("p-stale"))
    );
}

#[test]
fn a_key_change_or_fork_still_refuses_all_three() {
    let store = store();
    for who in ["p-changed", "p-forked"] {
        let unresolved = IntentError::SafetyUnresolved(ParticipantId::new(who));
        assert_eq!(human(&store, start(who)).unwrap_err(), unresolved);
        assert_eq!(human(&store, group(&["p-stale", who])).unwrap_err(), unresolved);
    }
    for (context, who) in [("c-changed", "p-changed"), ("c-forked", "p-forked")] {
        assert_eq!(
            check_start_call(&store, &ContextId::new(context)),
            Err(IntentError::SafetyUnresolved(ParticipantId::new(who)))
        );
    }
}
