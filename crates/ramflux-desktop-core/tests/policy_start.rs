// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for starting a direct conversation: only the attended human, only
//! with an accepted contact whose key is not in a changed state, never with
//! this account's own identity, under a fresh id, with a first message.

mod common;

use common::*;
use ramflux_desktop_core::*;

fn person(id: &str, link: ContactLink, safety: SafetyState) -> DomainEvent {
    let mut p = participant(id, ParticipantKind::Human, &format!("Name {id}"), true);
    p.link = link;
    p.safety = safety;
    DomainEvent::ParticipantUpserted(p)
}

fn store() -> Store {
    let mut feed = Feed::new();
    feed.ok(1, person("p-ana", ContactLink::Accepted, SafetyState::Unverified));
    feed.ok(1, person("p-bo", ContactLink::Accepted, SafetyState::Verified));
    feed.ok(1, person("p-cy", ContactLink::Accepted, SafetyState::KeyChanged));
    feed.ok(1, person("p-dee", ContactLink::Accepted, SafetyState::Forked));
    feed.ok(1, person("p-eve", ContactLink::Outgoing, SafetyState::Unverified));
    feed.ok(1, person("p-fin", ContactLink::Blocked, SafetyState::Unverified));
    feed.ok(
        1,
        DomainEvent::AccountUpdated(Account {
            self_participant_id: Some(ParticipantId::new("p-me")),
            ..account("acct-main")
        }),
    );
    feed.ok(1, person("p-me", ContactLink::Accepted, SafetyState::Verified));
    feed.ok(1, DomainEvent::ContextUpserted(context("c-existing", false)));
    feed.store
}

fn account(id: &str) -> Account {
    serde_json::from_value(serde_json::json!({
        "id": id, "label": "Main", "lock_state": "Unlocked",
    }))
    .expect("account")
}

fn start(participant: &str, context: &str, text: &str) -> IntentKind {
    IntentKind::StartConversation {
        context_id: ContextId::new(context),
        participant_id: ParticipantId::new(participant),
        text: text.to_owned(),
    }
}

fn human(store: &Store, kind: IntentKind) -> Result<Vec<Effect>, IntentError> {
    handle_intent(store, Intent::human(kind), Timestamp(10))
}

#[test]
fn an_accepted_contact_gets_the_first_message_with_their_name() {
    let store = store();
    for who in ["p-ana", "p-bo"] {
        let effects = human(&store, start(who, "c-new", "Hello")).expect("allowed");
        let [Effect::Call { method, body }] = effects.as_slice() else {
            panic!("one call expected, got {effects:?}");
        };
        assert_eq!(method, methods::CONVERSATION_START);
        assert_eq!(body["context_id"], "c-new");
        assert_eq!(body["participant_id"], who);
        assert_eq!(body["text"], "Hello");
        assert_eq!(body["title"], format!("Name {who}"));
        assert_eq!(body["proposed_by"], "Human");
    }
}

#[test]
fn starts_are_refused_with_the_reason() {
    let store = store();
    let refused = |kind| human(&store, kind).expect_err("refused");
    assert_eq!(
        refused(start("p-cy", "c-new", "x")),
        IntentError::SafetyUnresolved(ParticipantId::new("p-cy"))
    );
    assert_eq!(
        refused(start("p-dee", "c-new", "x")),
        IntentError::SafetyUnresolved(ParticipantId::new("p-dee"))
    );
    assert!(matches!(
        refused(start("p-eve", "c-new", "x")),
        IntentError::ContactLinkState { link: ContactLink::Outgoing, .. }
    ));
    assert!(matches!(
        refused(start("p-fin", "c-new", "x")),
        IntentError::ContactLinkState { link: ContactLink::Blocked, .. }
    ));
    assert_eq!(
        refused(start("p-zed", "c-new", "x")),
        IntentError::UnknownParticipant(ParticipantId::new("p-zed"))
    );
    assert_eq!(
        refused(start("p-me", "c-new", "x")),
        IntentError::SelfParticipant(ParticipantId::new("p-me"))
    );
    assert_eq!(
        refused(start("p-ana", "c-existing", "x")),
        IntentError::ContextExists(ContextId::new("c-existing"))
    );
    assert_eq!(refused(start("p-ana", "c-new", "  ")), IntentError::Empty("message text"));
    assert_eq!(refused(start("p-ana", " ", "x")), IntentError::Empty("conversation id"));
}

#[test]
fn agents_cannot_start_conversations() {
    let mut feed = Feed::new();
    feed.ok(1, person("p-ana", ContactLink::Accepted, SafetyState::Verified));
    feed.ok(
        1,
        DomainEvent::ParticipantUpserted(participant(
            "p-bot",
            ParticipantKind::Agent,
            "Bot",
            false,
        )),
    );
    let intent = Intent::agent(ParticipantId::new("p-bot"), start("p-ana", "c-new", "hi"));
    assert_eq!(
        handle_intent(&feed.store, intent, Timestamp(10)).expect_err("refused"),
        IntentError::AgentForbidden("start a conversation")
    );
}

#[test]
fn the_draft_check_matches_the_policy() {
    let store = store();
    assert!(check_start_conversation(&store, &ParticipantId::new("p-ana")).is_ok());
    assert!(check_start_conversation(&store, &ParticipantId::new("p-cy")).is_err());
    assert!(check_start_conversation(&store, &ParticipantId::new("p-eve")).is_err());
}
