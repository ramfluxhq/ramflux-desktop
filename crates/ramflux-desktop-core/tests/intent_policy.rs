// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Intent validation and policy.

mod common;

use std::marker::PhantomData;

use common::*;
use ramflux_desktop_core::*;
use serde::Serialize;

fn store_with_grants() -> Store {
    let mut feed = Feed::new();
    feed.ok(1, DomainEvent::ContextUpserted(context("c-1", false)));
    feed.ok(
        1,
        DomainEvent::ParticipantUpserted(participant(
            "p-agent",
            ParticipantKind::Agent,
            "Orbit",
            false,
        )),
    );
    feed.ok(
        1,
        DomainEvent::ParticipantUpserted(participant(
            "p-human",
            ParticipantKind::Human,
            "Ari",
            true,
        )),
    );
    for (id, risk) in [("g-low", Risk::Low), ("g-high", Risk::High), ("g-crit", Risk::Critical)] {
        feed.ok(2, DomainEvent::GrantRequested(grant(id, risk)));
    }
    feed.ok(2, DomainEvent::GrantRequested(grant("g-done", Risk::Low)));
    feed.ok(
        3,
        DomainEvent::GrantResolved {
            grant_id: GrantId::new("g-done"),
            state: GrantState::Approved,
        },
    );
    feed.ok(4, added(moment("m-s", "c-1", 4, surface("s-1"), Lifecycle::InProgress)));
    feed.ok(
        5,
        added(moment(
            "m-contact",
            "c-1",
            5,
            MomentKind::ContactRequest(ContactRequestPayload {
                from: ParticipantId::new("p-human"),
                note: String::new(),
            }),
            Lifecycle::NeedsMe,
        )),
    );
    feed.ok(
        6,
        DomainEvent::AccountUpdated(Account {
            id: AccountId::new("a-1"),
            label: "Personal".to_owned(),
            kind: AccountKind::HumanInteractive,
            node_id: None,
            device_id: None,
            lock_state: LockState::Locked,
            self_participant_id: None,
        }),
    );
    feed.store
}

const NOW: Timestamp = Timestamp(10);

fn handle_intent_now(store: &Store, intent: Intent) -> Result<Vec<Effect>, IntentError> {
    handle_intent(store, intent, NOW)
}

fn approve(grant: &str) -> IntentKind {
    IntentKind::Approve { grant_id: GrantId::new(grant) }
}

fn held(intent: Intent) -> Intent {
    intent.confirmed_by_human(ConfirmationIssuer::new().issue_hold(1_200).expect("a full hold"))
}

fn single(effects: Vec<Effect>) -> Effect {
    assert_eq!(effects.len(), 1, "expected exactly one effect");
    effects.into_iter().next().expect("one effect")
}

#[test]
fn high_risk_approve_without_confirmation_requires_confirmation() {
    let store = store_with_grants();
    for grant in ["g-high", "g-crit"] {
        let effect = single(handle_intent_now(&store, Intent::human(approve(grant))).expect("ok"));
        let Effect::RequireConfirmation { intent } = effect else {
            panic!("expected RequireConfirmation for {grant}");
        };
        assert!(matches!(intent.kind, IntentKind::Approve { .. }));
        assert!(!intent.is_confirmed());
    }
}

#[test]
fn high_risk_approve_with_confirmation_calls_source() {
    let store = store_with_grants();
    let effect =
        single(handle_intent_now(&store, held(Intent::human(approve("g-high")))).expect("ok"));
    let Effect::Call { method, body } = effect else { panic!("expected Call") };
    assert_eq!(method, methods::GRANT_APPROVE);
    assert_eq!(body["grant_id"], "g-high");
    assert_eq!(body["args_hash"], "hash-g-high");
    assert_eq!(body["confirmed_by"]["gesture"], "Hold");
    assert_eq!(body["proposed_by"], "Human");
}

#[test]
fn low_risk_approve_needs_no_confirmation() {
    let store = store_with_grants();
    let effect = single(handle_intent_now(&store, Intent::human(approve("g-low"))).expect("ok"));
    assert_eq!(effect.method(), Some(methods::GRANT_APPROVE));
}

#[test]
fn agent_cannot_self_confirm_high_risk() {
    let store = store_with_grants();
    let agent = Intent::agent(ParticipantId::new("p-agent"), approve("g-high"));
    let effect = single(handle_intent_now(&store, agent).expect("ok"));
    assert!(matches!(effect, Effect::RequireConfirmation { .. }));

    let low = Intent::agent(ParticipantId::new("p-agent"), approve("g-low"));
    let Effect::Call { body, .. } = single(handle_intent_now(&store, low).expect("ok")) else {
        panic!("expected Call");
    };
    assert_eq!(body["proposed_by"]["Agent"], "p-agent");
    assert!(body["confirmed_by"].is_null());
}

#[test]
fn unknown_or_non_agent_actor_is_rejected() {
    let store = store_with_grants();
    for id in ["p-ghost", "p-human"] {
        let intent = Intent::agent(ParticipantId::new(id), approve("g-low"));
        assert_eq!(
            handle_intent_now(&store, intent).err(),
            Some(IntentError::UnknownAgent(ParticipantId::new(id)))
        );
    }
}

#[test]
fn approving_a_decided_grant_is_an_error() {
    let store = store_with_grants();
    let result = handle_intent_now(&store, held(Intent::human(approve("g-done"))));
    assert_eq!(result.err(), Some(IntentError::GrantNotPending(GrantId::new("g-done"))));
    let missing = handle_intent_now(
        &store,
        Intent::human(IntentKind::Deny { grant_id: GrantId::new("g-x") }),
    );
    assert_eq!(missing.err(), Some(IntentError::UnknownGrant(GrantId::new("g-x"))));
}

#[test]
fn handling_intents_never_mutates_the_store() {
    let store = store_with_grants();
    let before = format!("{store:?}");
    let intents = [
        approve("g-high"),
        approve("g-low"),
        IntentKind::SendMessage { context_id: ContextId::new("c-1"), text: "hi".into() },
        IntentKind::PinContext { context_id: ContextId::new("c-1") },
        IntentKind::AcceptContact { moment_id: MomentId::new("m-contact") },
    ];
    for kind in intents {
        let _ = handle_intent_now(&store, Intent::human(kind));
    }
    assert_eq!(format!("{store:?}"), before);
}

#[test]
fn surface_action_is_validated_against_the_surface() {
    let store = store_with_grants();
    let action = |id: &str, surface: &str| {
        Intent::human(IntentKind::SurfaceAction {
            moment_id: MomentId::new("m-s"),
            surface_id: SurfaceId::new(surface),
            action_id: id.to_owned(),
            values: serde_json::json!({}),
        })
    };
    let ok = single(handle_intent_now(&store, action("run", "s-1")).expect("ok"));
    assert_eq!(ok.method(), Some(methods::SURFACE_ACTION));
    assert!(matches!(
        handle_intent_now(&store, action("export", "s-1")),
        Err(IntentError::ActionDisabled { .. })
    ));
    assert!(matches!(
        handle_intent_now(&store, action("run", "s-other")),
        Err(IntentError::UnknownAction { .. })
    ));
}

#[test]
fn message_and_pin_and_contact_validation() {
    let store = store_with_grants();
    let empty = IntentKind::SendMessage { context_id: ContextId::new("c-1"), text: "  ".into() };
    assert_eq!(
        handle_intent_now(&store, Intent::human(empty)).err(),
        Some(IntentError::Empty("message text"))
    );
    let unpin = IntentKind::UnpinContext { context_id: ContextId::new("c-1") };
    assert_eq!(
        handle_intent_now(&store, Intent::human(unpin)).err(),
        Some(IntentError::PinUnchanged(ContextId::new("c-1")))
    );
    let accept = IntentKind::AcceptContact { moment_id: MomentId::new("m-contact") };
    let effect = single(handle_intent_now(&store, Intent::human(accept)).expect("ok"));
    assert_eq!(effect.method(), Some(methods::CONTACT_ACCEPT));
    let wrong = IntentKind::RejectContact { moment_id: MomentId::new("m-s") };
    assert!(matches!(
        handle_intent_now(&store, Intent::human(wrong)),
        Err(IntentError::WrongMomentKind { expected: "ContactRequest", .. })
    ));
}

#[test]
fn unlock_passes_the_secret_only_to_the_source() {
    let store = store_with_grants();
    let unlock = || IntentKind::Unlock {
        account_id: AccountId::new("a-1"),
        secret: SecretInput::new("correct horse"),
    };
    let effect = single(handle_intent_now(&store, Intent::human(unlock())).expect("ok"));
    let Effect::SecretCall { method, body, secrets } = effect else {
        panic!("expected SecretCall")
    };
    assert_eq!(method, methods::ACCOUNT_UNLOCK);
    assert_eq!(body["account_id"], "a-1");
    assert!(!body.to_string().contains("correct horse"), "the body never holds the secret");
    assert_eq!(secrets.len(), 1);
    assert_eq!(secrets[0].expose(), "correct horse");

    let agent = Intent::agent(ParticipantId::new("p-agent"), unlock());
    assert!(matches!(handle_intent_now(&store, agent), Err(IntentError::AgentForbidden(_))));
}

#[test]
fn secret_input_debug_is_redacted() {
    let intent = Intent::human(IntentKind::Unlock {
        account_id: AccountId::new("a-1"),
        secret: SecretInput::new("correct horse"),
    });
    let debug = format!("{intent:?}");
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains("correct horse"));
    assert_eq!(format!("{:?}", SecretInput::new("pin-1234")), "<redacted>");
}

// Compile-time check that `SecretInput` does not implement `Serialize`.
// The inherent const on `Probe<T>` exists only when `T: Serialize` and takes
// precedence over the blanket trait const, which reports false.
trait NotSerializeFallback {
    const IS_SERIALIZE: bool = false;
}
impl<T: ?Sized> NotSerializeFallback for T {}
struct Probe<T: ?Sized>(PhantomData<T>);
impl<T: ?Sized + Serialize> Probe<T> {
    const IS_SERIALIZE: bool = true;
}
const _: () = assert!(!<Probe<SecretInput>>::IS_SERIALIZE, "SecretInput must not be Serialize");
const _: () = assert!(<Probe<String>>::IS_SERIALIZE, "probe must detect Serialize");

#[test]
fn secret_input_is_not_serialize() {
    const { assert!(!<Probe<SecretInput>>::IS_SERIALIZE) };
    const { assert!(<Probe<AccountId>>::IS_SERIALIZE) };
}
