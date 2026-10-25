// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Contact requests in the store (listing and updates, one per peer and
//! direction) and in policy (who may answer what, Block's hold, the
//! request setting).

mod common;

use common::*;
use ramflux_desktop_core::*;

fn request(
    id: &str,
    peer: &str,
    direction: RequestDirection,
    state: RequestState,
) -> ContactRequest {
    ContactRequest {
        id: id.to_owned(),
        peer: ParticipantId::new(peer),
        direction,
        state,
        note: None,
        safety_number: Some("12345".to_owned()),
        held_messages: 1,
        preview: Some("hi".to_owned()),
        expires_at: None,
    }
}

fn feed() -> Feed {
    let mut feed = Feed::new();
    for id in ["p-bo", "p-cy", "p-di"] {
        feed.ok(
            1,
            DomainEvent::ParticipantUpserted(participant(id, ParticipantKind::Human, id, false)),
        );
    }
    feed.ok(
        1,
        DomainEvent::ContactRequestsListed(vec![
            request("r-cy", "p-cy", RequestDirection::Incoming, RequestState::Pending),
            request("r-bo", "p-bo", RequestDirection::Incoming, RequestState::Pending),
        ]),
    );
    feed
}

fn pending(store: &Store) -> Vec<&str> {
    selectors::pending_contact_requests(store).iter().map(|r| r.id.as_str()).collect()
}

fn answer(id: &str, answer: RequestAnswer) -> IntentKind {
    IntentKind::AnswerContactRequest { request_id: id.to_owned(), answer }
}

#[test]
fn a_listing_replaces_the_pending_incoming_requests_and_keeps_answered_ones() {
    let mut feed = feed();
    assert_eq!(pending(&feed.store), ["r-bo", "r-cy"], "ordered by name");
    let declined = request("r-bo", "p-bo", RequestDirection::Incoming, RequestState::Declined);
    assert!(feed.ok(2, DomainEvent::ContactRequestUpserted(declined.clone())).contact_requests);
    // The same update again changes nothing.
    assert!(!feed.ok(2, DomainEvent::ContactRequestUpserted(declined)).contact_requests);
    let own = request("r-di", "p-di", RequestDirection::Outgoing, RequestState::Pending);
    feed.ok(2, DomainEvent::ContactRequestUpserted(own));
    // The daemon lists only r-cy now: r-bo (answered) stays as it ended,
    // the outgoing request is not touched by an incoming listing.
    feed.ok(
        3,
        DomainEvent::ContactRequestsListed(vec![request(
            "r-cy",
            "p-cy",
            RequestDirection::Incoming,
            RequestState::Pending,
        )]),
    );
    assert_eq!(pending(&feed.store), ["r-cy"]);
    let bo = feed.store.contact_request(&ParticipantId::new("p-bo"), RequestDirection::Incoming);
    assert_eq!(bo.map(|r| r.state), Some(RequestState::Declined));
    let di = selectors::outgoing_contact_request(&feed.store, &ParticipantId::new("p-di"));
    assert_eq!(di.map(|r| r.state), Some(RequestState::Pending));
    feed.ok(4, DomainEvent::ContactRequestsListed(Vec::new()));
    assert!(pending(&feed.store).is_empty());
    feed.ok(5, DomainEvent::ContactRequestPolicyUpdated(RequestPolicy::AutoAccept));
    assert_eq!(feed.store.request_policy(), Some(RequestPolicy::AutoAccept));
}

#[test]
fn answers_go_to_pending_incoming_requests_and_block_needs_the_hold() {
    let mut feed = feed();
    let effects = handle_intent(
        &feed.store,
        Intent::human(answer("r-cy", RequestAnswer::Accept)),
        Timestamp(0),
    )
    .expect("accept");
    assert!(matches!(&effects[..], [Effect::Call { method, body }]
        if method == methods::CONTACT_REQUEST_ACCEPT && body["request_id"] == "r-cy"
            && body["participant_id"] == "p-cy"));
    let effects = handle_intent(
        &feed.store,
        Intent::human(answer("r-cy", RequestAnswer::Decline)),
        Timestamp(0),
    )
    .expect("decline");
    assert!(
        matches!(&effects[..], [Effect::Call { method, .. }] if method == methods::CONTACT_REQUEST_DECLINE)
    );
    let effects = handle_intent(
        &feed.store,
        Intent::human(answer("r-cy", RequestAnswer::Block)),
        Timestamp(0),
    )
    .expect("block");
    assert!(matches!(&effects[..], [Effect::RequireConfirmation { .. }]), "{effects:?}");
    assert!(matches!(
        handle_intent(
            &feed.store,
            Intent::human(answer("r-zz", RequestAnswer::Accept)),
            Timestamp(0)
        ),
        Err(IntentError::UnknownContactRequest(_))
    ));
    assert!(
        handle_intent(
            &feed.store,
            Intent::agent(ParticipantId::new("p-bo"), answer("r-cy", RequestAnswer::Accept)),
            Timestamp(0)
        )
        .is_err()
    );
    // An outgoing request is not this account's to answer; an answered one
    // is not pending.
    let own = request("r-di", "p-di", RequestDirection::Outgoing, RequestState::Pending);
    feed.ok(2, DomainEvent::ContactRequestUpserted(own));
    assert!(matches!(
        handle_intent(
            &feed.store,
            Intent::human(answer("r-di", RequestAnswer::Accept)),
            Timestamp(0)
        ),
        Err(IntentError::UnknownContactRequest(_))
    ));
    let done = request("r-cy", "p-cy", RequestDirection::Incoming, RequestState::Accepted);
    feed.ok(3, DomainEvent::ContactRequestUpserted(done));
    assert!(matches!(
        handle_intent(
            &feed.store,
            Intent::human(answer("r-cy", RequestAnswer::Decline)),
            Timestamp(0)
        ),
        Err(IntentError::ContactRequestNotPending(_))
    ));
}

#[test]
fn the_request_setting_is_human_only_and_adding_a_requester_is_refused() {
    let feed = feed();
    let set = || IntentKind::SetContactRequestPolicy { policy: RequestPolicy::AutoAccept };
    let effects = handle_intent(&feed.store, Intent::human(set()), Timestamp(0)).expect("set");
    assert!(matches!(&effects[..], [Effect::Call { method, body }]
        if method == methods::CONTACT_REQUESTS_POLICY && body["contact_requests"] == "auto_accept"));
    assert!(
        handle_intent(&feed.store, Intent::agent(ParticipantId::new("p-bo"), set()), Timestamp(0))
            .is_err()
    );
    assert!(matches!(
        handle_intent(
            &feed.store,
            Intent::human(IntentKind::AddContact { participant_id: ParticipantId::new("p-cy") }),
            Timestamp(0)
        ),
        Err(IntentError::ContactLinkState { link: ContactLink::Incoming, .. })
    ));
}
