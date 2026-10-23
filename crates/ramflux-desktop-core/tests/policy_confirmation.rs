// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy added for the app shell: grant expiry, high-risk surface actions,
//! governance, safety verification, handoff, and human confirmation.

mod common;

use common::*;
use ramflux_desktop_core::*;

const NOW: Timestamp = Timestamp(1_000);

fn agent(kind: IntentKind) -> Intent {
    Intent::agent(ParticipantId::new("p-agent"), kind)
}

fn held(intent: Intent) -> Intent {
    intent.confirmed_by_human(ConfirmationIssuer::new().issue_hold(1_200).expect("a full hold"))
}

fn store() -> Store {
    let mut feed = Feed::new();
    for (id, kind, name) in [
        ("p-agent", ParticipantKind::Agent, "Orbit"),
        ("p-ana", ParticipantKind::Human, "Ana"),
        ("p-bo", ParticipantKind::Human, "Bo"),
        ("p-cy", ParticipantKind::Human, "Cy"),
        ("d-desk", ParticipantKind::Device, "Desk"),
    ] {
        feed.ok(1, DomainEvent::ParticipantUpserted(participant(id, kind, name, true)));
    }
    let mut group = context("c-group", false);
    group.participants = vec![ParticipantId::new("p-ana"), ParticipantId::new("p-bo")];
    feed.ok(1, DomainEvent::ContextUpserted(group));
    let mut direct = context("c-direct", false);
    direct.kind = ContextKind::Direct;
    feed.ok(1, DomainEvent::ContextUpserted(direct));

    let mut expiring = grant("g-old", Risk::Low);
    expiring.expiry = Some(NOW);
    feed.ok(2, DomainEvent::GrantRequested(expiring));
    let mut later = grant("g-later", Risk::Low);
    later.expiry = Some(Timestamp(NOW.millis() + 1));
    feed.ok(2, DomainEvent::GrantRequested(later));

    let mut risky = surface("s-risky");
    if let MomentKind::AgentSurface(payload) = &mut risky {
        payload.actions.push(SurfaceAction {
            id: "wipe".to_owned(),
            label: "Wipe".to_owned(),
            risk: Risk::Critical,
            disabled_reason: None,
        });
    }
    feed.ok(3, added(moment("m-risky", "c-group", 3, risky, Lifecycle::Done)));
    let handoff = MomentKind::Handoff(HandoffPayload {
        from_device: ParticipantId::new("d-desk"),
        to_device: None,
        summary: "Continue".to_owned(),
    });
    feed.ok(4, added(moment("m-handoff", "c-direct", 4, handoff, Lifecycle::NeedsMe)));
    let mut verified = participant("p-cy", ParticipantKind::Human, "Cy", true);
    verified.safety = SafetyState::Verified;
    feed.ok(5, DomainEvent::ParticipantUpserted(verified));
    feed.store
}

fn effects(store: &Store, intent: Intent) -> Vec<Effect> {
    handle_intent(store, intent, NOW).expect("intent accepted")
}

fn is_confirmation(effects: &[Effect]) -> bool {
    matches!(effects, [Effect::RequireConfirmation { .. }])
}

fn call_body(effects: Vec<Effect>) -> (String, JsonValue) {
    match effects.into_iter().next() {
        Some(Effect::Call { method, body }) => (method, body),
        other => panic!("expected a call, got {other:?}"),
    }
}

fn governance(op: GovernanceOp) -> IntentKind {
    IntentKind::Governance { context_id: ContextId::new("c-group"), op }
}

#[test]
fn expired_grants_are_refused_at_expiry() {
    let store = store();
    for kind in [
        IntentKind::Approve { grant_id: GrantId::new("g-old") },
        IntentKind::Deny { grant_id: GrantId::new("g-old") },
    ] {
        assert_eq!(
            handle_intent(&store, Intent::human(kind), NOW).err(),
            Some(IntentError::GrantExpired(GrantId::new("g-old")))
        );
    }
    let later = IntentKind::Approve { grant_id: GrantId::new("g-later") };
    assert_eq!(effects(&store, Intent::human(later)).len(), 1);
}

#[test]
fn critical_surface_action_needs_human_confirmation() {
    let store = store();
    let shown = store.surface_hash(&MomentId::new("m-risky")).cloned().expect("surface hash");
    let wipe = || IntentKind::SurfaceAction {
        moment_id: MomentId::new("m-risky"),
        surface_id: SurfaceId::new("s-risky"),
        action_id: "wipe".to_owned(),
        values: JsonValue::Null,
        surface_hash: shown.clone(),
        version: 0,
    };
    assert!(is_confirmation(&effects(&store, Intent::human(wipe()))));
    assert!(is_confirmation(&effects(&store, agent(wipe()))), "an agent proposal waits for a hold");
    let (method, body) = call_body(effects(&store, held(Intent::human(wipe()))));
    assert_eq!(method, methods::SURFACE_ACTION);
    assert_eq!(body["confirmed_by"]["gesture"], "Hold");

    let run = IntentKind::SurfaceAction {
        moment_id: MomentId::new("m-risky"),
        surface_id: SurfaceId::new("s-risky"),
        action_id: "run".to_owned(),
        values: JsonValue::Null,
        surface_hash: shown.clone(),
        version: 0,
    };
    assert!(!is_confirmation(&effects(&store, Intent::human(run))));
}

#[test]
fn kick_ban_and_set_role_need_confirmation_add_and_invite_do_not() {
    let store = store();
    let bo = || ParticipantId::new("p-bo");
    let high = [
        GovernanceOp::Kick { participant_id: bo() },
        GovernanceOp::Ban { participant_id: bo() },
        GovernanceOp::SetRole { participant_id: bo(), role: Role::Admin },
    ];
    for op in high {
        assert!(is_confirmation(&effects(&store, Intent::human(governance(op.clone())))));
        assert!(is_confirmation(&effects(&store, agent(governance(op.clone())))));
        let (method, body) = call_body(effects(&store, held(agent(governance(op.clone())))));
        assert_eq!(method, op.method());
        assert_eq!(body["participant_id"], "p-bo");
        assert_eq!(body["context_id"], "c-group");
        assert_eq!(body["proposed_by"]["Agent"], "p-agent", "the proposer is kept");
        assert_eq!(body["confirmed_by"]["held_ms"], 1_200);
    }
    let add =
        GovernanceOp::AddMember { participant_id: ParticipantId::new("p-cy"), role: Role::Member };
    let (method, body) = call_body(effects(&store, Intent::human(governance(add))));
    assert_eq!(method, methods::GROUP_ADD_MEMBER);
    assert_eq!(body["role"], "Member");
    let invite =
        GovernanceOp::CreateInvite { participant_id: ParticipantId::new("p-cy"), role: Role::Bot };
    let (method, _) = call_body(effects(&store, Intent::human(governance(invite))));
    assert_eq!(method, methods::GROUP_CREATE_INVITE);
    let remove = GovernanceOp::RemoveMember { participant_id: bo() };
    let (method, _) = call_body(effects(&store, Intent::human(governance(remove))));
    assert_eq!(method, methods::GROUP_REMOVE_MEMBER);
}

#[test]
fn governance_validates_context_and_membership() {
    let store = store();
    let refuse = |kind: IntentKind| handle_intent(&store, Intent::human(kind), NOW).err();
    let direct = IntentKind::Governance {
        context_id: ContextId::new("c-direct"),
        op: GovernanceOp::CreateInvite { participant_id: "p-cy".into(), role: Role::Member },
    };
    assert_eq!(refuse(direct), Some(IntentError::NotAGroup(ContextId::new("c-direct"))));
    let kick_outsider = governance(GovernanceOp::Kick { participant_id: "p-cy".into() });
    assert!(matches!(refuse(kick_outsider), Some(IntentError::NotAMember { .. })));
    let add_member =
        governance(GovernanceOp::AddMember { participant_id: "p-ana".into(), role: Role::Member });
    assert!(matches!(refuse(add_member), Some(IntentError::AlreadyMember { .. })));
    let ghost = governance(GovernanceOp::Ban { participant_id: "p-ghost".into() });
    assert_eq!(refuse(ghost), Some(IntentError::UnknownParticipant("p-ghost".into())));
}

#[test]
fn safety_verification_is_human_only_and_not_repeated() {
    let store = store();
    let verify =
        |id: &str| IntentKind::VerifySafetyNumber { participant_id: ParticipantId::new(id) };
    let (method, body) = call_body(effects(&store, Intent::human(verify("p-ana"))));
    assert_eq!(method, methods::SAFETY_VERIFY);
    assert_eq!(body["participant_id"], "p-ana");
    assert!(matches!(
        handle_intent(&store, agent(verify("p-ana")), NOW),
        Err(IntentError::AgentForbidden(_))
    ));
    assert_eq!(
        handle_intent(&store, Intent::human(verify("p-cy")), NOW).err(),
        Some(IntentError::AlreadyVerified(ParticipantId::new("p-cy")))
    );
}

#[test]
fn handoff_answers_target_open_handoff_moments() {
    let store = store();
    let accept = IntentKind::AcceptHandoff { moment_id: MomentId::new("m-handoff") };
    let (method, body) = call_body(effects(&store, Intent::human(accept)));
    assert_eq!(method, methods::HANDOFF_ACCEPT);
    assert_eq!(body["from_device"], "d-desk");
    let decline = IntentKind::DeclineHandoff { moment_id: MomentId::new("m-handoff") };
    assert_eq!(call_body(effects(&store, Intent::human(decline))).0, methods::HANDOFF_DECLINE);
    let by_agent = IntentKind::AcceptHandoff { moment_id: MomentId::new("m-handoff") };
    assert!(matches!(
        handle_intent(&store, agent(by_agent), NOW),
        Err(IntentError::AgentForbidden(_))
    ));
    let wrong = IntentKind::AcceptHandoff { moment_id: MomentId::new("m-risky") };
    assert!(matches!(
        handle_intent(&store, Intent::human(wrong), NOW),
        Err(IntentError::WrongMomentKind { expected: "Handoff", .. })
    ));
}

#[test]
fn human_confirmation_keeps_the_proposer() {
    let proposed = agent(IntentKind::Approve { grant_id: GrantId::new("g-1") });
    assert!(!proposed.is_confirmed());
    let confirmed = held(proposed);
    assert_eq!(confirmed.proposed_by, Actor::Agent(ParticipantId::new("p-agent")));
    assert_eq!(confirmed.confirmed_by.as_ref().map(HumanConfirmation::held_ms), Some(1_200));
    assert!(confirmed.is_confirmed());
}

#[test]
fn secret_input_exposes_by_reference() {
    let secret = SecretInput::new("pin-1234");
    assert_eq!(secret.expose(), "pin-1234");
    assert!(SecretInput::new("").is_empty());
}

#[test]
fn a_confirmation_cannot_be_forged_through_serde() {
    let forged = serde_json::json!({
        "proposed_by": { "Agent": "p-agent" },
        "confirmed_by": { "gesture": "Hold", "held_ms": 5_000 },
        "kind": { "RevokeGrant": { "grant_id": "g-old" } },
    });
    let intent: Intent = serde_json::from_value(forged).expect("intent");
    assert!(!intent.is_confirmed(), "deserialization never yields a confirmation");
    assert!(ConfirmationIssuer::new().issue_hold(MIN_HOLD_MS - 1).is_none());
}

#[test]
fn methods_the_backend_cannot_perform_are_refused_before_any_hold() {
    let mut feed = Feed::new();
    let mut ctx = context("c-1", false);
    ctx.kind = ContextKind::Direct;
    ctx.participants = vec![ParticipantId::new("p-agent")];
    feed.ok(
        1,
        DomainEvent::ParticipantUpserted(participant(
            "p-agent",
            ParticipantKind::Agent,
            "Orbit",
            false,
        )),
    );
    feed.ok(1, DomainEvent::ContextUpserted(ctx));
    let start = || IntentKind::StartTask {
        context_id: ContextId::new("c-1"),
        assignee: ParticipantId::new("p-agent"),
        text: "x".into(),
    };
    assert!(handle_intent(&feed.store, Intent::human(start()), NOW).is_ok());
    let unsupported = Capabilities {
        unsupported: [(methods::TASK_START.to_owned(), "needs K3".to_owned())]
            .into_iter()
            .collect(),
    };
    feed.ok(2, DomainEvent::CapabilitiesUpdated(unsupported));
    assert_eq!(feed.store.unsupported_reason(methods::TASK_START), Some("needs K3"));
    assert!(matches!(
        handle_intent(&feed.store, Intent::human(start()), NOW),
        Err(IntentError::UnsupportedByBackend { method: methods::TASK_START, .. })
    ));
}
