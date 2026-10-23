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
            node_id: None,
            device_id: None,
            lock_state: LockState::Locked,
        }),
    );
    feed.store
}

fn approve(grant: &str, confirmed: bool) -> IntentKind {
    IntentKind::Approve { grant_id: GrantId::new(grant), confirmed }
}

fn single(effects: Vec<Effect>) -> Effect {
    assert_eq!(effects.len(), 1, "expected exactly one effect");
    effects.into_iter().next().expect("one effect")
}

#[test]
fn high_risk_approve_without_confirmation_requires_confirmation() {
    let store = store_with_grants();
    for grant in ["g-high", "g-crit"] {
        let effect =
            single(handle_intent(&store, Intent::human(approve(grant, false))).expect("ok"));
        let Effect::RequireConfirmation { intent } = effect else {
            panic!("expected RequireConfirmation for {grant}");
        };
        assert!(matches!(intent.kind, IntentKind::Approve { confirmed: false, .. }));
    }
}

#[test]
fn high_risk_approve_with_confirmation_calls_source() {
    let store = store_with_grants();
    let effect = single(handle_intent(&store, Intent::human(approve("g-high", true))).expect("ok"));
    let Effect::Call { method, body } = effect else { panic!("expected Call") };
    assert_eq!(method, methods::GRANT_APPROVE);
    assert_eq!(body["grant_id"], "g-high");
    assert_eq!(body["args_hash"], "hash-g-high");
    assert_eq!(body["confirmed"], true);
    assert_eq!(body["actor"], "Human");
}

#[test]
fn low_risk_approve_needs_no_confirmation() {
    let store = store_with_grants();
    let effect = single(handle_intent(&store, Intent::human(approve("g-low", false))).expect("ok"));
    assert_eq!(effect.method(), Some(methods::GRANT_APPROVE));
}

#[test]
fn agent_cannot_self_confirm_high_risk() {
    let store = store_with_grants();
    let agent = Intent {
        actor: Actor::Agent(ParticipantId::new("p-agent")),
        kind: approve("g-high", true),
    };
    let effect = single(handle_intent(&store, agent).expect("ok"));
    assert!(matches!(effect, Effect::RequireConfirmation { .. }));

    let low = Intent {
        actor: Actor::Agent(ParticipantId::new("p-agent")),
        kind: approve("g-low", false),
    };
    let Effect::Call { body, .. } = single(handle_intent(&store, low).expect("ok")) else {
        panic!("expected Call");
    };
    assert_eq!(body["actor"]["Agent"], "p-agent");
}

#[test]
fn unknown_or_non_agent_actor_is_rejected() {
    let store = store_with_grants();
    for id in ["p-ghost", "p-human"] {
        let intent =
            Intent { actor: Actor::Agent(ParticipantId::new(id)), kind: approve("g-low", false) };
        assert_eq!(
            handle_intent(&store, intent).err(),
            Some(IntentError::UnknownAgent(ParticipantId::new(id)))
        );
    }
}

#[test]
fn approving_a_decided_grant_is_an_error() {
    let store = store_with_grants();
    let result = handle_intent(&store, Intent::human(approve("g-done", true)));
    assert_eq!(result.err(), Some(IntentError::GrantNotPending(GrantId::new("g-done"))));
    let missing =
        handle_intent(&store, Intent::human(IntentKind::Deny { grant_id: GrantId::new("g-x") }));
    assert_eq!(missing.err(), Some(IntentError::UnknownGrant(GrantId::new("g-x"))));
}

#[test]
fn handling_intents_never_mutates_the_store() {
    let store = store_with_grants();
    let before = format!("{store:?}");
    let intents = [
        approve("g-high", true),
        approve("g-high", false),
        IntentKind::SendMessage { context_id: ContextId::new("c-1"), text: "hi".into() },
        IntentKind::PinContext { context_id: ContextId::new("c-1") },
        IntentKind::AcceptContact { moment_id: MomentId::new("m-contact") },
    ];
    for kind in intents {
        let _ = handle_intent(&store, Intent::human(kind));
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
    let ok = single(handle_intent(&store, action("run", "s-1")).expect("ok"));
    assert_eq!(ok.method(), Some(methods::SURFACE_ACTION));
    assert!(matches!(
        handle_intent(&store, action("export", "s-1")),
        Err(IntentError::ActionDisabled { .. })
    ));
    assert!(matches!(
        handle_intent(&store, action("run", "s-other")),
        Err(IntentError::UnknownAction { .. })
    ));
}

#[test]
fn message_and_pin_and_contact_validation() {
    let store = store_with_grants();
    let empty = IntentKind::SendMessage { context_id: ContextId::new("c-1"), text: "  ".into() };
    assert_eq!(
        handle_intent(&store, Intent::human(empty)).err(),
        Some(IntentError::Empty("message text"))
    );
    let unpin = IntentKind::UnpinContext { context_id: ContextId::new("c-1") };
    assert_eq!(
        handle_intent(&store, Intent::human(unpin)).err(),
        Some(IntentError::PinUnchanged(ContextId::new("c-1")))
    );
    let accept = IntentKind::AcceptContact { moment_id: MomentId::new("m-contact") };
    let effect = single(handle_intent(&store, Intent::human(accept)).expect("ok"));
    assert_eq!(effect.method(), Some(methods::CONTACT_ACCEPT));
    let wrong = IntentKind::RejectContact { moment_id: MomentId::new("m-s") };
    assert!(matches!(
        handle_intent(&store, Intent::human(wrong)),
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
    let effect = single(handle_intent(&store, Intent::human(unlock())).expect("ok"));
    let Effect::Unlock { account_id, secret } = effect else { panic!("expected Unlock") };
    assert_eq!(account_id, AccountId::new("a-1"));
    assert_eq!(secret.expose(), "correct horse");

    let agent = Intent { actor: Actor::Agent(ParticipantId::new("p-agent")), kind: unlock() };
    assert!(matches!(handle_intent(&store, agent), Err(IntentError::AgentForbidden(_))));
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
