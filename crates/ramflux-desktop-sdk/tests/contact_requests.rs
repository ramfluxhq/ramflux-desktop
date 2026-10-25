// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Contact requests through the SDK source and a fake daemon with the
//! CONTACT-02 request API (contract-first, docs/contract.md): an incoming
//! request arrives on the stream with its note, safety number and held
//! messages; Accept, Decline and Block reach the daemon by request id (an
//! accept reads the conversations again, where the held messages land);
//! this account's own request shows on the contact, is sent again only when
//! asked, shows a decline, and a re-request during the decline's cooldown
//! is explained; the request setting is read and set; and a daemon without
//! requests has every request method declared unsupported.

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Weak};

use common::*;
use ramflux_desktop_core::{
    AccountId, ContextId, FailureCode, Intent, IntentError, IntentKind, ParticipantId,
    RequestAnswer, RequestDirection, RequestPolicy, RequestState, Source, Store, SyncMode,
    Timestamp, handle_intent, methods, selectors,
};
use ramflux_desktop_test::fake_contact_requests::FakeContactRequests;
use serde_json::{Value, json};

const ACCOUNT: &str = "acct-main";

struct World {
    fake: Arc<FakeBus>,
    requests: Arc<FakeContactRequests>,
    path: PathBuf,
}

impl World {
    fn start(name: &str, available: bool) -> Self {
        let path = socket_path(&format!("requests-{name}"));
        let requests = Arc::new(FakeContactRequests::default());
        requests.set_available(available);
        let handler = requests.clone();
        let fake = Arc::new(FakeBus::start(&path, move |method, account, body| {
            if let Some(reply) = handler.handle(method, account, body) {
                return reply;
            }
            if method == "contact.list" {
                // The contact row carries this account's own request.
                let mut listed = frames()["contact.list"].clone();
                for row in listed["contacts"].as_array_mut().into_iter().flatten() {
                    let peer = row["target_id"].as_str().unwrap_or_default().to_owned();
                    if let Some(state) = handler.own_state(ACCOUNT, &peer) {
                        row["principal_id"] = json!(peer);
                        row["request_state"] = json!(state);
                    }
                }
                return Reply::Ok(listed);
            }
            fixture(method)
        }));
        fake.enable_stream();
        let bus: Weak<FakeBus> = Arc::downgrade(&fake);
        requests.set_publisher(Box::new(move |account, topic, body| {
            if let Some(bus) = bus.upgrade() {
                bus.publish(account, topic, body);
            }
        }));
        Self { fake, requests, path }
    }

    fn requests(&self, method: &str) -> Vec<Value> {
        self.fake.seen().into_iter().filter(|s| s.method == method).map(|s| s.body).collect()
    }

    fn stop(self) {
        if let Ok(fake) = Arc::try_unwrap(self.fake) {
            fake.shutdown();
        }
    }
}

fn live(world: &World) -> Fed {
    let hub = ramflux_desktop_sdk::SdkHub::start(config(&world.path)).expect("bridge");
    let mut account = Fed::new(hub.account_source(AccountId::new(ACCOUNT)));
    account.until("the account is live", |s, _| {
        s.context(&ContextId::new("dm-ana")).is_some()
            && s.sync_status().is_some_and(|sync| sync.mode == SyncMode::Stream)
    });
    account
}

fn dispatch(account: &mut Fed, kind: IntentKind) -> Result<(), IntentError> {
    for effect in handle_intent(&account.store, Intent::human(kind), Timestamp(0))? {
        account.source.submit(effect);
    }
    Ok(())
}

fn pending(store: &Store) -> Vec<String> {
    selectors::pending_contact_requests(store).iter().map(|r| r.peer.to_string()).collect()
}

fn state_of(store: &Store, peer: &str, direction: RequestDirection) -> Option<RequestState> {
    store.contact_request(&ParticipantId::new(peer), direction).map(|r| r.state)
}

#[test]
fn an_incoming_request_shows_its_held_messages_and_is_accepted() {
    let world = World::start("accept", true);
    let mut account = live(&world);
    assert!(account.store.unsupported_reason(methods::CONTACT_REQUEST_ACCEPT).is_none());
    assert_eq!(account.store.request_policy(), Some(RequestPolicy::Manual));

    let id = world.requests.incoming(ACCOUNT, "principal-cy", Some("we met at the meetup"));
    account.until("the request", |s, _| pending(s) == ["principal-cy"]);
    world.requests.hold(&id, "hi, it's Cy");
    // The event carries the count; the preview comes with the list it
    // calls for.
    account.until("the held message", |s, _| {
        selectors::pending_contact_requests(s)
            .first()
            .is_some_and(|r| r.held_messages == 1 && r.preview.is_some())
    });
    let request = selectors::pending_contact_requests(&account.store)[0].clone();
    assert_eq!(request.note.as_deref(), Some("we met at the meetup"));
    assert_eq!(request.preview.as_deref(), Some("hi, it's Cy"));
    // The daemon's twelve digit groups, joined.
    assert_eq!(
        request.safety_number.as_deref(),
        Some("05173 88420 61937 24459 90314 77502 13579 24680 11223 44556 77889 99001")
    );
    assert!(account.store.participant(&ParticipantId::new("principal-cy")).is_some());
    // Adding someone who asked is answering their request instead.
    assert!(matches!(
        handle_intent(
            &account.store,
            Intent::human(IntentKind::AddContact { participant_id: "principal-cy".into() }),
            Timestamp(0)
        ),
        Err(IntentError::ContactLinkState { .. })
    ));

    let directories = world.requests("conversation.directory").len();
    dispatch(
        &mut account,
        IntentKind::AnswerContactRequest { request_id: id.clone(), answer: RequestAnswer::Accept },
    )
    .expect("allowed");
    account.until("accepted", |s, _| {
        state_of(s, "principal-cy", RequestDirection::Incoming) == Some(RequestState::Accepted)
    });
    assert_eq!(world.requests("contact.accept"), [json!({ "request_id": id })]);
    assert!(pending(&account.store).is_empty());
    // The held message is delivered into the conversation: read it again.
    account.until("the conversations read again", |_, _| {
        world.requests("conversation.directory").len() > directories
    });
    assert_eq!(world.requests.delivered().len(), 1);
    // A request answered already cannot be answered again.
    assert!(matches!(
        handle_intent(
            &account.store,
            Intent::human(IntentKind::AnswerContactRequest {
                request_id: id,
                answer: RequestAnswer::Decline
            }),
            Timestamp(0)
        ),
        Err(IntentError::ContactRequestNotPending(_))
    ));
    world.stop();
}

#[test]
fn decline_and_block_reach_the_daemon_and_block_needs_the_hold() {
    let world = World::start("decline", true);
    let mut account = live(&world);
    let declined = world.requests.incoming(ACCOUNT, "principal-cy", None);
    let blocked = world.requests.incoming(ACCOUNT, "principal-bo", None);
    account.until("both requests", |s, _| pending(s).len() == 2);
    dispatch(
        &mut account,
        IntentKind::AnswerContactRequest {
            request_id: declined.clone(),
            answer: RequestAnswer::Decline,
        },
    )
    .expect("allowed");
    account.until("declined", |s, _| {
        state_of(s, "principal-cy", RequestDirection::Incoming) == Some(RequestState::Declined)
    });
    assert_eq!(world.requests("contact.decline"), [json!({ "request_id": declined })]);

    let block = Intent::human(IntentKind::AnswerContactRequest {
        request_id: blocked.clone(),
        answer: RequestAnswer::Block,
    });
    let effects = handle_intent(&account.store, block, Timestamp(0)).expect("allowed");
    assert!(
        matches!(effects.as_slice(), [ramflux_desktop_core::Effect::RequireConfirmation { .. }]),
        "{effects:?}"
    );
    let held = Intent::human(IntentKind::AnswerContactRequest {
        request_id: blocked.clone(),
        answer: RequestAnswer::Block,
    })
    .confirmed_by_human(
        ramflux_desktop_core::ConfirmationIssuer::new().issue_hold(1_200).expect("a full hold"),
    );
    for effect in handle_intent(&account.store, held, Timestamp(0)).expect("held") {
        account.source.submit(effect);
    }
    account.until("blocked", |s, _| {
        state_of(s, "principal-bo", RequestDirection::Incoming) == Some(RequestState::Blocked)
    });
    assert_eq!(world.requests("contact.block"), [json!({ "request_id": blocked })]);
    world.stop();
}

#[test]
fn this_accounts_own_request_shows_until_the_peer_answers() {
    let world = World::start("outgoing", true);
    let id = world.requests.outgoing(ACCOUNT, "principal-ana");
    let mut account = live(&world);
    account.until("waiting for Ana", |s, _| {
        selectors::outgoing_contact_request(s, &ParticipantId::new("principal-ana"))
            .is_some_and(|r| r.state == RequestState::Pending)
    });
    world.requests.peer_answers(&id, "accepted");
    account.until("Ana accepted", |s, _| {
        selectors::outgoing_contact_request(s, &ParticipantId::new("principal-ana")).is_none()
    });
    world.stop();
}

#[test]
fn asking_again_resends_a_pending_request_and_a_decline_cools_down() {
    let world = World::start("again", true);
    let id = world.requests.outgoing(ACCOUNT, "principal-ana");
    let mut account = live(&world);
    let ana = ParticipantId::new("principal-ana");
    account.until("waiting for Ana", |s, _| {
        selectors::outgoing_contact_request(s, &ana)
            .is_some_and(|r| r.state == RequestState::Pending)
    });
    // Asked again explicitly: the same request goes out again.
    dispatch(&mut account, IntentKind::AddContact { participant_id: ana.clone() })
        .expect("allowed");
    account.until("sent again", |_, _| world.requests.sent(&id) == 2);
    assert_eq!(world.requests("contact.add"), [json!({ "principal_id": "principal-ana" })]);

    // Ana declines: shown as declined, and nothing is sent on its own.
    world.requests.peer_answers(&id, "declined");
    account.until("declined", |s, _| {
        selectors::outgoing_contact_request(s, &ana)
            .is_some_and(|r| r.state == RequestState::Declined)
    });
    for _ in 0..10 {
        account.pump();
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    assert_eq!(world.requests("contact.add").len(), 1, "no automatic retry");

    // Asking again while the decline cools down is refused, plainly.
    dispatch(&mut account, IntentKind::AddContact { participant_id: ana.clone() })
        .expect("allowed");
    account.until("the cooldown explained", |s, _| {
        s.failures().iter().any(|f| {
            f.method == methods::CONTACT_ADD
                && f.message.starts_with("backend.contact.request_cooldown ")
        })
    });

    // After the cooldown a new request goes out.
    world.requests.cooldown_over(&id);
    dispatch(&mut account, IntentKind::AddContact { participant_id: ana.clone() })
        .expect("allowed");
    account.until("a new request", |s, _| {
        selectors::outgoing_contact_request(s, &ana)
            .is_some_and(|r| r.state == RequestState::Pending && r.id != id)
    });
    world.stop();
}

#[test]
fn the_request_setting_is_read_and_set() {
    let world = World::start("policy", true);
    let mut account = live(&world);
    dispatch(
        &mut account,
        IntentKind::SetContactRequestPolicy { policy: RequestPolicy::AutoAccept },
    )
    .expect("allowed");
    account.until("auto accept", |s, _| s.request_policy() == Some(RequestPolicy::AutoAccept));
    assert_eq!(
        world.requests("contact.requests.policy.set"),
        [json!({ "contact_requests": "auto_accept" })]
    );
    assert_eq!(world.requests.policy(ACCOUNT), "auto_accept");
    // Now a request is accepted by the daemon at once.
    world.requests.incoming(ACCOUNT, "principal-cy", None);
    account.until("accepted at once", |s, _| {
        state_of(s, "principal-cy", RequestDirection::Incoming) == Some(RequestState::Accepted)
    });
    assert!(pending(&account.store).is_empty());
    world.stop();
}

#[test]
fn a_daemon_without_requests_has_every_request_method_unsupported() {
    let world = World::start("old", false);
    let mut account = live(&world);
    for method in ramflux_desktop_sdk::capabilities::REQUEST_METHODS {
        assert_eq!(
            account.store.unsupported_reason(method),
            Some(ramflux_desktop_sdk::capabilities::reason::CONTACT_REQUESTS),
            "{method}"
        );
    }
    // An effect that reaches the source anyway fails without a request.
    account.source.submit(ramflux_desktop_core::Effect::Call {
        method: methods::CONTACT_REQUESTS_POLICY.to_owned(),
        body: json!({ "contact_requests": "manual", "proposed_by": "Human", "confirmed_by": null }),
    });
    account.until("refused", |s, _| {
        s.failures().iter().any(|f| {
            f.method == methods::CONTACT_REQUESTS_POLICY
                && f.code == FailureCode::UnsupportedByBackend
        })
    });
    assert!(world.requests("contact.requests.policy.set").is_empty());
    world.stop();
}
