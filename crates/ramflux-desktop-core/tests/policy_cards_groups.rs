// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for contact cards (adding a contact from a card, exporting this
//! account's own), for creating and renaming a group, and the store's
//! record of the exported card.

mod common;

use common::*;
use ramflux_desktop_core::*;

fn person(
    id: &str,
    link: ContactLink,
    safety: SafetyState,
    reachable: Option<bool>,
) -> DomainEvent {
    let mut p = participant(id, ParticipantKind::Human, &format!("Name {id}"), true);
    p.link = link;
    p.safety = safety;
    p.reachable = reachable;
    DomainEvent::ParticipantUpserted(p)
}

fn store() -> Store {
    let mut feed = Feed::new();
    feed.ok(1, person("p-ana", ContactLink::Accepted, SafetyState::Unverified, Some(true)));
    feed.ok(1, person("p-bo", ContactLink::Accepted, SafetyState::Verified, None));
    feed.ok(1, person("p-cy", ContactLink::Accepted, SafetyState::KeyChanged, Some(true)));
    feed.ok(1, person("p-dee", ContactLink::Accepted, SafetyState::Unverified, Some(false)));
    feed.ok(1, person("p-eve", ContactLink::Outgoing, SafetyState::Unverified, Some(true)));
    feed.ok(1, person("p-agent", ContactLink::Accepted, SafetyState::Unverified, Some(true)));
    let mut agent = participant("p-bot", ParticipantKind::Agent, "Bot", true);
    agent.link = ContactLink::Accepted;
    feed.ok(1, DomainEvent::ParticipantUpserted(agent));
    feed.ok(1, DomainEvent::ContextUpserted(context("g-existing", false)));
    let mut direct = context("dm-ana", false);
    direct.kind = ContextKind::Direct;
    feed.ok(1, DomainEvent::ContextUpserted(direct));
    feed.store
}

fn human(store: &Store, kind: IntentKind) -> Result<Vec<Effect>, IntentError> {
    handle_intent(store, Intent::human(kind), Timestamp(10))
}

fn agent(store: &Store, kind: IntentKind) -> Result<Vec<Effect>, IntentError> {
    handle_intent(store, Intent::agent(ParticipantId::new("p-bot"), kind), Timestamp(10))
}

fn call(effects: &[Effect]) -> (&str, &serde_json::Value) {
    match effects {
        [Effect::Call { method, body }] => (method.as_str(), body),
        other => panic!("one call expected, got {other:?}"),
    }
}

fn create(title: &str, members: &[&str]) -> IntentKind {
    IntentKind::CreateGroup {
        context_id: ContextId::new("g-new"),
        title: title.to_owned(),
        members: members.iter().map(|m| ParticipantId::new(*m)).collect(),
    }
}

#[test]
fn a_card_is_imported_trimmed_by_the_human_only() {
    let store = store();
    let effects = human(
        &store,
        IntentKind::AddContactCard { card: "  ramflux:contact/abc\nsig  ".to_owned() },
    )
    .expect("allowed");
    let (method, body) = call(&effects);
    assert_eq!(method, methods::CONTACT_CARD_IMPORT);
    assert_eq!(body["card"], "ramflux:contact/abc\nsig");
    assert_eq!(body["proposed_by"], "Human");
    let refused = |card: &str| {
        human(&store, IntentKind::AddContactCard { card: card.to_owned() }).expect_err("refused")
    };
    assert_eq!(refused("   "), IntentError::InvalidContactCard(ContactCardProblem::Empty));
    assert_eq!(
        refused(&"c".repeat(MAX_CONTACT_CARD_LEN + 1)),
        IntentError::InvalidContactCard(ContactCardProblem::TooLong)
    );
    assert_eq!(refused("card\u{0}"), IntentError::InvalidContactCard(ContactCardProblem::Control));
    assert!(matches!(
        agent(&store, IntentKind::AddContactCard { card: "ramflux:x".to_owned() }),
        Err(IntentError::AgentForbidden(_))
    ));
}

#[test]
fn the_own_card_is_exported_for_the_human_and_kept_in_the_store() {
    let mut store = store();
    let effects = human(&store, IntentKind::ExportContactCard).expect("allowed");
    assert_eq!(call(&effects).0, methods::CONTACT_CARD_EXPORT);
    assert!(matches!(
        agent(&store, IntentKind::ExportContactCard),
        Err(IntentError::AgentForbidden(_))
    ));
    assert_eq!(store.own_contact_card(), None);
    let card = ContactCard { text: "card".to_owned(), link: None };
    let env = EventEnvelope::new(
        store.last_seq() + 1,
        Timestamp(2),
        DomainEvent::OwnContactCardUpdated(card.clone()),
    );
    let changes = store.apply(&env).expect("applies");
    assert!(changes.source_status);
    assert_eq!(store.own_contact_card(), Some(&card));
    let again = EventEnvelope::new(
        store.last_seq() + 1,
        Timestamp(3),
        DomainEvent::OwnContactCardUpdated(card),
    );
    assert!(store.apply(&again).expect("applies").is_empty(), "the same card changes nothing");
}

#[test]
fn a_group_is_created_with_its_name_and_each_member_once() {
    let store = store();
    let effects =
        human(&store, create("  Weekend trip ", &["p-ana", "p-bo", "p-ana"])).expect("ok");
    let (method, body) = call(&effects);
    assert_eq!(method, methods::GROUP_CREATE);
    assert_eq!(body["context_id"], "g-new");
    assert_eq!(body["title"], "Weekend trip");
    assert_eq!(body["members"], serde_json::json!(["p-ana", "p-bo"]));
    assert_eq!(
        check_create_group(&store, " Trip ", &[ParticipantId::new("p-bo")]),
        Ok(("Trip", vec![ParticipantId::new("p-bo")]))
    );
}

#[test]
fn group_creation_is_refused_with_the_reason() {
    let store = store();
    let refused = |kind| human(&store, kind).expect_err("refused");
    assert_eq!(
        refused(create(" ", &["p-ana"])),
        IntentError::InvalidGroup(GroupProblem::EmptyTitle)
    );
    assert_eq!(
        refused(create(&"t".repeat(MAX_GROUP_TITLE_LEN + 1), &["p-ana"])),
        IntentError::InvalidGroup(GroupProblem::TitleTooLong)
    );
    assert_eq!(
        refused(create("a\u{7}b", &["p-ana"])),
        IntentError::InvalidGroup(GroupProblem::TitleControl)
    );
    assert_eq!(refused(create("Trip", &[])), IntentError::InvalidGroup(GroupProblem::NoMembers));
    let many: Vec<String> = (0..=MAX_GROUP_MEMBERS).map(|i| format!("p-{i}")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    assert_eq!(
        refused(create("Trip", &many)),
        IntentError::InvalidGroup(GroupProblem::TooManyMembers)
    );
    assert_eq!(
        refused(create("Trip", &["p-ana", "p-who"])),
        IntentError::UnknownParticipant(ParticipantId::new("p-who"))
    );
    assert!(matches!(
        refused(create("Trip", &["p-eve"])),
        IntentError::ContactLinkState { link: ContactLink::Outgoing, .. }
    ));
    assert_eq!(
        refused(create("Trip", &["p-cy"])),
        IntentError::SafetyUnresolved(ParticipantId::new("p-cy"))
    );
    assert_eq!(
        refused(create("Trip", &["p-dee"])),
        IntentError::NotReachable(ParticipantId::new("p-dee"))
    );
    let existing = IntentKind::CreateGroup {
        context_id: ContextId::new("g-existing"),
        title: "Trip".to_owned(),
        members: vec![ParticipantId::new("p-ana")],
    };
    assert_eq!(refused(existing), IntentError::ContextExists(ContextId::new("g-existing")));
    assert!(matches!(
        agent(&store, create("Trip", &["p-ana"])),
        Err(IntentError::AgentForbidden(_))
    ));
}

#[test]
fn an_unreachable_contact_cannot_be_messaged_yet() {
    let store = store();
    assert_eq!(
        check_start_conversation(&store, &ParticipantId::new("p-dee")),
        Err(IntentError::NotReachable(ParticipantId::new("p-dee")))
    );
    assert_eq!(
        check_start_conversation(&store, &ParticipantId::new("p-bo")),
        Ok(()),
        "not reported"
    );
}

#[test]
fn only_a_group_is_renamed_and_never_to_a_blank_name() {
    let store = store();
    let rename = |context: &str, title: &str| IntentKind::RenameContext {
        context_id: ContextId::new(context),
        title: title.to_owned(),
    };
    let effects = human(&store, rename("g-existing", " Crew ")).expect("allowed");
    let (method, body) = call(&effects);
    assert_eq!(method, methods::CONTEXT_RENAME);
    assert_eq!(
        (body["context_id"].as_str(), body["title"].as_str()),
        (Some("g-existing"), Some("Crew"))
    );
    assert_eq!(
        human(&store, rename("dm-ana", "Ana")).expect_err("refused"),
        IntentError::NotAGroup(ContextId::new("dm-ana"))
    );
    assert_eq!(
        human(&store, rename("g-existing", "  ")).expect_err("refused"),
        IntentError::InvalidGroup(GroupProblem::EmptyTitle)
    );
    assert_eq!(
        human(&store, rename("g-gone", "Crew")).expect_err("refused"),
        IntentError::UnknownContext(ContextId::new("g-gone"))
    );
}
