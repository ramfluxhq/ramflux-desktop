// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for reveals, revocations, objects, entry intents and settings.

mod common;

use common::*;
use ramflux_desktop_core::*;

const NOW: Timestamp = Timestamp(100);

fn held(intent: Intent) -> Intent {
    intent.confirmed_by_human(ConfirmationIssuer::new().issue_hold(1_200).expect("a full hold"))
}

fn node(id: &str, pin_source: PinSource, confirmed: bool) -> Node {
    Node {
        id: NodeId::new(id),
        endpoints: vec!["quic://node.example.test:7443".to_owned()],
        fingerprint: Some("SHA256:aa".to_owned()),
        pin_source,
        health: NodeHealth::Unknown,
        capabilities: Vec::new(),
        expires_at: None,
        confirmed,
        builtin: false,
        enabled: true,
        is_default: false,
        probe: None,
    }
}

fn object(id: &str, direction: Direction, transfer: TransferState) -> Object {
    Object {
        id: ObjectId::new(id),
        name: Some(format!("{id}.bin")),
        size: Some(10),
        direction,
        transfer,
        saved_to: None,
        preview: None,
    }
}

fn store() -> Store {
    let mut feed = Feed::new();
    feed.ok(
        1,
        DomainEvent::ParticipantUpserted(participant(
            "p-agent",
            ParticipantKind::Agent,
            "Orbit",
            false,
        )),
    );
    feed.ok(1, DomainEvent::ContextUpserted(context("c-1", false)));
    let masked = MomentKind::PrivacyReveal(PrivacyRevealPayload {
        label: "Account number".to_owned(),
        revealed: None,
        content: None,
    });
    feed.ok(2, added(moment("m-mask", "c-1", 2, masked, Lifecycle::Pending)));
    feed.ok(3, DomainEvent::NodeUpserted(node("n-home", PinSource::Invite, true)));
    feed.ok(3, DomainEvent::NodeUpserted(node("n-tofu", PinSource::Manual, false)));
    feed.ok(3, DomainEvent::NodeUpserted(node("n-invite", PinSource::Invite, false)));
    feed.ok(
        4,
        DomainEvent::AccountUpdated(Account {
            id: AccountId::new("a-1"),
            label: "Personal".to_owned(),
            kind: AccountKind::HumanInteractive,
            node_id: Some(NodeId::new("n-home")),
            device_id: None,
            lock_state: LockState::Unlocked,
            self_participant_id: None,
        }),
    );
    feed.ok(5, DomainEvent::GrantRequested(grant("g-ok", Risk::Low)));
    feed.ok(
        6,
        DomainEvent::GrantResolved { grant_id: GrantId::new("g-ok"), state: GrantState::Approved },
    );
    feed.ok(
        7,
        DomainEvent::StandingApprovalUpserted(StandingApproval {
            id: StandingApprovalId::new("sa-1"),
            scope: "tool.calendar.read".to_owned(),
            max_risk: Risk::Medium,
            expiry: None,
            uses: 0,
            revoked: false,
        }),
    );
    for (id, this_device) in [("dev-me", true), ("dev-old", false)] {
        feed.ok(
            8,
            DomainEvent::DeviceUpserted(Device {
                id: DeviceId::new(id),
                account_id: AccountId::new("a-1"),
                label: id.to_owned(),
                this_device,
                revoked: false,
                last_seen: None,
            }),
        );
    }
    feed.ok(
        9,
        DomainEvent::ObjectUpserted(object("o-queued", Direction::Inbound, TransferState::Queued)),
    );
    feed.ok(
        9,
        DomainEvent::ObjectUpserted(object("o-done", Direction::Inbound, TransferState::Complete)),
    );
    feed.ok(
        9,
        DomainEvent::ObjectUpserted(object(
            "o-failed",
            Direction::Outbound,
            TransferState::Failed { reason: "reset".to_owned() },
        )),
    );
    feed.store
}

fn decide(store: &Store, intent: Intent) -> Result<Effect, IntentError> {
    handle_intent(store, intent, NOW).map(|mut effects| effects.remove(0))
}

fn needs_hold(store: &Store, kind: IntentKind) -> bool {
    matches!(decide(store, Intent::human(kind)), Ok(Effect::RequireConfirmation { .. }))
}

#[test]
fn reveals_and_revocations_need_a_hold_and_carry_the_audit_pair() {
    let store = store();
    let kinds = || {
        [
            IntentKind::Reveal { moment_id: MomentId::new("m-mask") },
            IntentKind::RevokeGrant { grant_id: GrantId::new("g-ok") },
            IntentKind::RevokeStandingApproval { approval_id: StandingApprovalId::new("sa-1") },
            IntentKind::RevokeDevice { device_id: DeviceId::new("dev-old") },
        ]
    };
    for kind in kinds() {
        assert!(needs_hold(&store, kind));
    }
    for kind in kinds() {
        let effect = decide(&store, held(Intent::human(kind))).expect("held intent passes");
        let body = effect.body().expect("call body");
        assert_eq!(body["proposed_by"], "Human");
        assert_eq!(body["confirmed_by"]["gesture"], "Hold");
    }
    assert_eq!(
        decide(
            &store,
            Intent::human(IntentKind::RevokeDevice { device_id: DeviceId::new("dev-me") })
        )
        .err(),
        Some(IntentError::ThisDevice(DeviceId::new("dev-me")))
    );
}

#[test]
fn object_actions_follow_the_transfer_state() {
    let store = store();
    let method = |kind| decide(&store, Intent::human(kind)).map(|e| e.method().map(str::to_owned));
    let pull = |id: &str| IntentKind::PullObject { object_id: ObjectId::new(id) };
    assert_eq!(method(pull("o-queued")), Ok(Some(methods::OBJECT_PULL.to_owned())));
    assert!(matches!(method(pull("o-done")), Err(IntentError::TransferState { .. })));
    let save = IntentKind::SaveObject { object_id: ObjectId::new("o-done"), path: "/tmp/x".into() };
    assert_eq!(method(save), Ok(Some(methods::OBJECT_SAVE.to_owned())));
    let retry = |id: &str| IntentKind::RetryTransfer { object_id: ObjectId::new(id) };
    assert_eq!(method(retry("o-failed")), Ok(Some(methods::OBJECT_RETRY.to_owned())));
    assert!(matches!(method(retry("o-queued")), Err(IntentError::TransferState { .. })));
}

#[test]
fn confirming_a_tofu_node_needs_a_hold_and_is_human_only() {
    let store = store();
    let confirm =
        |id: &str, pin_source| IntentKind::ConfirmNode { node_id: NodeId::new(id), pin_source };
    assert!(needs_hold(&store, confirm("n-tofu", PinSource::Manual)));
    let effect =
        decide(&store, held(Intent::human(confirm("n-tofu", PinSource::Manual)))).expect("ok");
    let body = effect.body().expect("body");
    assert_eq!(effect.method(), Some(methods::NODE_CONFIRM));
    assert_eq!(body["fingerprint"], "SHA256:aa");
    assert_eq!(body["confirmed_by"]["gesture"], "Hold");
    assert!(
        !needs_hold(&store, confirm("n-invite", PinSource::Invite)),
        "an invite pin is not TOFU"
    );
    assert_eq!(
        decide(&store, Intent::human(confirm("n-invite", PinSource::Manual))).err(),
        Some(IntentError::PinMismatch(NodeId::new("n-invite")))
    );
    assert_eq!(
        decide(&store, Intent::human(confirm("n-home", PinSource::Invite))).err(),
        Some(IntentError::NodeAlreadyConfirmed(NodeId::new("n-home")))
    );
    let by_agent =
        Intent::agent(ParticipantId::new("p-agent"), confirm("n-tofu", PinSource::Manual));
    assert!(matches!(decide(&store, by_agent), Err(IntentError::AgentForbidden(_))));
}

#[test]
fn account_creation_needs_a_confirmed_node_and_keeps_the_secret_out_of_the_body() {
    let store = store();
    let create = |node: &str| IntentKind::CreateAccount {
        kind: AccountKind::HeadlessAi,
        label: "Night shift".into(),
        node_id: NodeId::new(node),
        secret: SecretInput::new("lantern river"),
    };
    assert_eq!(
        decide(&store, Intent::human(create("n-tofu"))).err(),
        Some(IntentError::NodeNotConfirmed(NodeId::new("n-tofu")))
    );
    let Ok(Effect::SecretCall { method, body, secrets }) =
        decide(&store, Intent::human(create("n-home")))
    else {
        panic!("expected a secret call");
    };
    assert_eq!(method, methods::ACCOUNT_CREATE);
    assert_eq!(body["kind"], "HeadlessAi");
    assert!(!body.to_string().contains("lantern"));
    assert_eq!(secrets.len(), 1);
    let by_agent = Intent::agent(ParticipantId::new("p-agent"), create("n-home"));
    assert!(matches!(decide(&store, by_agent), Err(IntentError::AgentForbidden(_))));
}

#[test]
fn backup_export_carries_its_passphrase_as_a_secret_and_is_human_only() {
    let store = store();
    let export = |passphrase: &str| IntentKind::ExportBackup {
        account_id: AccountId::new("a-1"),
        path: "/tmp/a.backup".into(),
        secret: SecretInput::new(passphrase),
    };
    assert_eq!(
        decide(&store, Intent::human(export(""))).err(),
        Some(IntentError::Empty("backup passphrase"))
    );
    let Ok(Effect::SecretCall { method, body, secrets }) =
        decide(&store, Intent::human(export("quiet harbor")))
    else {
        panic!("expected a secret call");
    };
    assert_eq!(method, methods::ACCOUNT_BACKUP_EXPORT);
    assert_eq!(body["path"], "/tmp/a.backup");
    assert!(!body.to_string().contains("quiet"), "the passphrase stays out of the body");
    assert_eq!(secrets.len(), 1);
    assert_eq!(secrets[0].expose(), "quiet harbor");
    let by_agent = Intent::agent(ParticipantId::new("p-agent"), export("quiet harbor"));
    assert!(matches!(decide(&store, by_agent), Err(IntentError::AgentForbidden(_))));
}

#[test]
fn node_settings_guard_bound_and_default_nodes() {
    let store = store();
    let refuse = |kind| decide(&store, Intent::human(kind)).err();
    assert_eq!(
        refuse(IntentKind::RemoveNode { node_id: NodeId::new("n-home") }),
        Some(IntentError::NodeBound(NodeId::new("n-home")))
    );
    assert_eq!(refuse(IntentKind::RemoveNode { node_id: NodeId::new("n-invite") }), None);
    assert_eq!(
        refuse(IntentKind::SetDefaultNode { node_id: NodeId::new("n-tofu") }),
        Some(IntentError::NodeNotConfirmed(NodeId::new("n-tofu")))
    );
    assert_eq!(refuse(IntentKind::ProbeNode { node_id: NodeId::new("n-home") }), None);
    assert_eq!(
        refuse(IntentKind::AddNode { url: "  ".into() }),
        Some(IntentError::Empty("node address"))
    );
}
