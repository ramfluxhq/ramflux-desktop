// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for contacts: adding one by id and every action on a contact link.

mod common;

use common::*;
use ramflux_desktop_core::*;
use serde_json::json;

fn linked(id: &str, link: ContactLink) -> DomainEvent {
    let mut p = participant(id, ParticipantKind::Human, id, link == ContactLink::Accepted);
    p.link = link;
    DomainEvent::ParticipantUpserted(p)
}

fn store() -> Store {
    let mut feed = Feed::new();
    feed.ok(1, linked("p-friend", ContactLink::Accepted));
    feed.ok(1, linked("p-asking", ContactLink::Incoming));
    feed.ok(1, linked("p-blocked", ContactLink::Blocked));
    feed.ok(1, linked("p-gone", ContactLink::Removed));
    feed.ok(
        1,
        DomainEvent::ParticipantUpserted(participant(
            "p-scenario",
            ParticipantKind::Human,
            "Ari",
            true,
        )),
    );
    feed.ok(
        1,
        DomainEvent::ParticipantUpserted(participant(
            "p-agent",
            ParticipantKind::Agent,
            "Orbit",
            false,
        )),
    );
    feed.store
}

fn human(store: &Store, kind: IntentKind) -> Result<Vec<Effect>, IntentError> {
    handle_intent(store, Intent::human(kind), Timestamp(10))
}

fn held(store: &Store, kind: IntentKind) -> Result<Vec<Effect>, IntentError> {
    let hold = ConfirmationIssuer::new().issue_hold(1_200).expect("a full hold");
    handle_intent(store, Intent::human(kind).confirmed_by_human(hold), Timestamp(10))
}

fn call(effects: &[Effect]) -> (&str, &JsonValue) {
    match effects {
        [Effect::Call { method, body }] => (method, body),
        other => panic!("one call expected, got {other:?}"),
    }
}

fn act(id: &str, action: ContactAction) -> IntentKind {
    IntentKind::Contact { participant_id: ParticipantId::new(id), action }
}

#[test]
fn adding_a_contact_calls_contact_add_with_the_typed_id() {
    let store = store();
    let add = |id: &str| IntentKind::AddContact { participant_id: ParticipantId::new(id) };
    let effects = human(&store, add("principal-new")).expect("allowed");
    let (method, body) = call(&effects);
    assert_eq!(method, methods::CONTACT_ADD);
    assert_eq!(body["participant_id"], "principal-new");
    assert_eq!(body["proposed_by"], "Human");
    assert_eq!(body["confirmed_by"], json!(null), "adding is low risk: no hold");
    // A removed link can be added again.
    assert!(human(&store, add("p-gone")).is_ok());
}

#[test]
fn adding_refuses_bad_ids_existing_links_and_agents() {
    let store = store();
    let add = |id: &str| IntentKind::AddContact { participant_id: ParticipantId::new(id) };
    for (id, problem) in [
        ("", ContactIdProblem::Empty),
        ("two words", ContactIdProblem::Spaces),
        ("tab\there", ContactIdProblem::Spaces),
    ] {
        assert!(
            matches!(human(&store, add(id)), Err(IntentError::InvalidContactId(p)) if p == problem),
            "{id:?}"
        );
    }
    let long = "x".repeat(MAX_CONTACT_ID_LEN + 1);
    assert!(matches!(
        human(&store, add(&long)),
        Err(IntentError::InvalidContactId(ContactIdProblem::TooLong))
    ));
    for (id, link) in [
        ("p-friend", ContactLink::Accepted),
        ("p-scenario", ContactLink::Accepted),
        ("p-blocked", ContactLink::Blocked),
        ("p-asking", ContactLink::Incoming),
    ] {
        assert!(
            matches!(human(&store, add(id)), Err(IntentError::ContactLinkState { link: l, .. }) if l == link),
            "{id}"
        );
    }
    let agent = Intent::agent(ParticipantId::new("p-agent"), add("principal-new"));
    assert!(matches!(
        handle_intent(&store, agent, Timestamp(10)),
        Err(IntentError::AgentForbidden(_))
    ));
}

#[test]
fn each_contact_action_maps_to_its_method_for_the_states_it_applies_to() {
    let store = store();
    for (id, action, method) in [
        ("p-asking", ContactAction::Accept, methods::CONTACT_ACCEPT),
        ("p-asking", ContactAction::Reject, methods::CONTACT_REJECT),
        ("p-blocked", ContactAction::Unblock, methods::CONTACT_UNBLOCK),
    ] {
        let effects = human(&store, act(id, action)).expect("allowed");
        let (m, body) = call(&effects);
        assert_eq!((m, body["participant_id"].as_str()), (method, Some(id)));
    }
    for (id, action) in [
        ("p-asking", ContactAction::Unblock),
        ("p-friend", ContactAction::Accept),
        ("p-friend", ContactAction::Reject),
        ("p-friend", ContactAction::Unblock),
        ("p-blocked", ContactAction::Block),
        ("p-gone", ContactAction::Remove),
        ("p-gone", ContactAction::Block),
    ] {
        assert!(
            matches!(human(&store, act(id, action)), Err(IntentError::ContactLinkState { .. })),
            "{id} {action:?}"
        );
    }
    assert!(matches!(
        human(&store, act("p-nobody", ContactAction::Block)),
        Err(IntentError::UnknownParticipant(_))
    ));
    // A scenario contact (contact flag, no link) is an accepted link.
    assert!(held(&store, act("p-scenario", ContactAction::Remove)).is_ok());
}

#[test]
fn blocking_and_removing_need_the_hold_and_carry_it() {
    let store = store();
    for (id, action, method) in [
        ("p-friend", ContactAction::Block, methods::CONTACT_BLOCK),
        ("p-friend", ContactAction::Remove, methods::CONTACT_REMOVE),
        ("p-blocked", ContactAction::Remove, methods::CONTACT_REMOVE),
        ("p-asking", ContactAction::Block, methods::CONTACT_BLOCK),
    ] {
        let pending = human(&store, act(id, action)).expect("allowed");
        assert!(
            matches!(pending.as_slice(), [Effect::RequireConfirmation { .. }]),
            "{action:?} needs the hold"
        );
        let effects = held(&store, act(id, action)).expect("allowed");
        let (m, body) = call(&effects);
        assert_eq!(m, method);
        assert_eq!(body["participant_id"], id);
        assert_eq!(body["confirmed_by"]["gesture"], "Hold");
    }
    // An agent may propose a block; only the human's hold sends it.
    let agent = Intent::agent(ParticipantId::new("p-agent"), act("p-friend", ContactAction::Block));
    let effects = handle_intent(&store, agent, Timestamp(10)).expect("proposal allowed");
    assert!(matches!(effects.as_slice(), [Effect::RequireConfirmation { .. }]));
}

#[test]
fn unsupported_contact_methods_are_refused_before_any_hold() {
    let mut store = store();
    let mut unsupported = std::collections::BTreeMap::new();
    unsupported.insert(methods::CONTACT_BLOCK.to_owned(), "backend.gap.test".to_owned());
    let at = store.last_seq() + 1;
    store
        .apply(&EventEnvelope::new(
            at,
            Timestamp(2),
            DomainEvent::CapabilitiesUpdated(Capabilities { unsupported }),
        ))
        .expect("capabilities");
    assert!(matches!(
        human(&store, act("p-friend", ContactAction::Block)),
        Err(IntentError::UnsupportedByBackend { method: methods::CONTACT_BLOCK, .. })
    ));
    assert!(contact_action_current(&store, &ParticipantId::new("p-friend"), ContactAction::Block));
    assert!(!contact_action_current(&store, &ParticipantId::new("p-gone"), ContactAction::Block));
}
