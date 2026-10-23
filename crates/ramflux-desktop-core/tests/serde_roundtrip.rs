// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Serde round-trips for every event, intent, and UI event variant.
//!
//! Each `*_name` function matches exhaustively, so adding a variant without a
//! sample here fails to compile or fails the coverage assertion.

mod common;

use std::collections::BTreeSet;

use common::*;
use ramflux_desktop_core::*;
use serde_json::json;

fn domain_event_name(event: &DomainEvent) -> &'static str {
    match event {
        DomainEvent::AccountUpdated(_) => "AccountUpdated",
        DomainEvent::NodeUpserted(_) => "NodeUpserted",
        DomainEvent::NodeHealthChanged { .. } => "NodeHealthChanged",
        DomainEvent::ParticipantUpserted(_) => "ParticipantUpserted",
        DomainEvent::ContextUpserted(_) => "ContextUpserted",
        DomainEvent::ContextRemoved { .. } => "ContextRemoved",
        DomainEvent::MomentAdded(_) => "MomentAdded",
        DomainEvent::MomentUpdated { .. } => "MomentUpdated",
        DomainEvent::SurfacePatched { .. } => "SurfacePatched",
        DomainEvent::ObjectUpserted(_) => "ObjectUpserted",
        DomainEvent::GrantRequested(_) => "GrantRequested",
        DomainEvent::GrantResolved { .. } => "GrantResolved",
        DomainEvent::TaskUpserted(_) => "TaskUpserted",
        DomainEvent::TaskStepChanged { .. } => "TaskStepChanged",
        DomainEvent::SourceStatusChanged(_) => "SourceStatusChanged",
    }
}
const DOMAIN_EVENT_VARIANTS: usize = 15;

fn moment_kind_name(kind: &MomentKind) -> &'static str {
    match kind {
        MomentKind::Message(_) => "Message",
        MomentKind::AgentSurface(_) => "AgentSurface",
        MomentKind::ObjectTransfer(_) => "ObjectTransfer",
        MomentKind::Intervention(_) => "Intervention",
        MomentKind::SafetyChange(_) => "SafetyChange",
        MomentKind::Governance(_) => "Governance",
        MomentKind::ContactRequest(_) => "ContactRequest",
        MomentKind::TaskProgress(_) => "TaskProgress",
        MomentKind::Conflict(_) => "Conflict",
        MomentKind::PrivacyReveal(_) => "PrivacyReveal",
        MomentKind::Handoff(_) => "Handoff",
        MomentKind::Fallback(_) => "Fallback",
        MomentKind::System(_) => "System",
    }
}
const MOMENT_KIND_VARIANTS: usize = 13;

fn intent_kind_name(kind: &IntentKind) -> &'static str {
    match kind {
        IntentKind::SendMessage { .. } => "SendMessage",
        IntentKind::AttachObject { .. } => "AttachObject",
        IntentKind::SurfaceAction { .. } => "SurfaceAction",
        IntentKind::Approve { .. } => "Approve",
        IntentKind::Deny { .. } => "Deny",
        IntentKind::Reveal { .. } => "Reveal",
        IntentKind::ResolveConflict { .. } => "ResolveConflict",
        IntentKind::StartTask { .. } => "StartTask",
        IntentKind::CancelTask { .. } => "CancelTask",
        IntentKind::PinContext { .. } => "PinContext",
        IntentKind::UnpinContext { .. } => "UnpinContext",
        IntentKind::AcceptContact { .. } => "AcceptContact",
        IntentKind::RejectContact { .. } => "RejectContact",
        IntentKind::VerifySafetyNumber { .. } => "VerifySafetyNumber",
        IntentKind::Governance { .. } => "Governance",
        IntentKind::AcceptHandoff { .. } => "AcceptHandoff",
        IntentKind::DeclineHandoff { .. } => "DeclineHandoff",
        IntentKind::Unlock { .. } => "Unlock",
    }
}
/// Serializable intent variants (all but `Unlock`).
const SERIALIZABLE_INTENT_VARIANTS: usize = 17;

fn ui_event_kind_name(kind: &UiEventKind) -> &'static str {
    match kind {
        UiEventKind::SubmitMessage { .. } => "SubmitMessage",
        UiEventKind::AttachFile { .. } => "AttachFile",
        UiEventKind::ActionInvoked { .. } => "ActionInvoked",
        UiEventKind::ApprovePressed { .. } => "ApprovePressed",
        UiEventKind::DenyPressed => "DenyPressed",
        UiEventKind::RevealPressed => "RevealPressed",
        UiEventKind::ConflictResolved { .. } => "ConflictResolved",
        UiEventKind::TaskRequested { .. } => "TaskRequested",
        UiEventKind::CancelPressed => "CancelPressed",
        UiEventKind::PinToggled { .. } => "PinToggled",
        UiEventKind::ContactAnswered { .. } => "ContactAnswered",
        UiEventKind::ExpandToggled { .. } => "ExpandToggled",
    }
}
const UI_EVENT_KIND_VARIANTS: usize = 12;

fn all_moment_kinds() -> Vec<MomentKind> {
    vec![
        message("hello"),
        surface("s-1"),
        MomentKind::ObjectTransfer(ObjectTransferPayload { object_id: ObjectId::new("o-1") }),
        MomentKind::Intervention(InterventionPayload {
            grant_id: GrantId::new("g-1"),
            risk: Risk::High,
            summary: "Run a shell command".to_owned(),
        }),
        MomentKind::SafetyChange(SafetyChangePayload {
            participant_id: ParticipantId::new("p-1"),
            change: SafetyChangeKind::KeyChanged,
        }),
        MomentKind::Governance(GovernancePayload {
            action: GovernanceAction::RoleChanged,
            target: ParticipantId::new("p-2"),
            role: Some(Role::Admin),
        }),
        MomentKind::ContactRequest(ContactRequestPayload {
            from: ParticipantId::new("p-3"),
            note: "We met at the workshop".to_owned(),
        }),
        MomentKind::TaskProgress(TaskProgressPayload { task_id: TaskId::new("t-1") }),
        MomentKind::Conflict(ConflictPayload {
            object_id: ObjectId::new("o-2"),
            local: ConflictSide { version: "v3".to_owned(), summary: "Local edit".to_owned() },
            remote: ConflictSide { version: "v4".to_owned(), summary: "Remote edit".to_owned() },
            resolution: Some(ConflictResolution::KeepBoth),
        }),
        MomentKind::PrivacyReveal(PrivacyRevealPayload {
            label: "Account number".to_owned(),
            revealed: Some(RevealRecord { by: ParticipantId::new("p-me"), at: Timestamp(9) }),
            content: Some("1234".to_owned()),
        }),
        MomentKind::Handoff(HandoffPayload {
            from_device: ParticipantId::new("d-laptop"),
            to_device: None,
            summary: "Continue the draft".to_owned(),
        }),
        MomentKind::Fallback(FallbackPayload {
            reason: "unsupported surface".to_owned(),
            text: "Open on a newer client".to_owned(),
        }),
        MomentKind::System(SystemPayload { level: SystemLevel::Warning, text: "Node slow".into() }),
    ]
}

fn all_domain_events() -> Vec<DomainEvent> {
    let mut events = vec![
        DomainEvent::AccountUpdated(Account {
            id: AccountId::new("a-1"),
            label: "Personal".to_owned(),
            node_id: Some(NodeId::new("n-1")),
            device_id: Some("dev-1".to_owned()),
            lock_state: LockState::Unlocked,
        }),
        DomainEvent::NodeUpserted(Node {
            id: NodeId::new("n-1"),
            endpoints: vec!["quic://node.example.test:7443".to_owned()],
            pin_source: PinSource::Invite,
            health: NodeHealth::Healthy,
        }),
        DomainEvent::NodeHealthChanged {
            node_id: NodeId::new("n-1"),
            health: NodeHealth::Degraded,
        },
        DomainEvent::ParticipantUpserted(participant("p-1", ParticipantKind::Agent, "Orbit", true)),
        DomainEvent::ContextUpserted(context("c-1", true)),
        DomainEvent::ContextRemoved { context_id: ContextId::new("c-9") },
        DomainEvent::MomentUpdated {
            moment_id: MomentId::new("m-1"),
            lifecycle: Some(Lifecycle::Done),
            kind: Some(message("edited")),
        },
        DomainEvent::SurfacePatched {
            moment_id: MomentId::new("m-2"),
            surface_id: SurfaceId::new("s-1"),
            ops: vec![
                SurfaceOp::AppendBlocks(vec![Block {
                    id: BlockId::new("b-2"),
                    kind: BlockKind::Progress,
                    data: json!({ "value": 0.5 }),
                }]),
                SurfaceOp::ReplaceBlock(Block {
                    id: BlockId::new("b-1"),
                    kind: BlockKind::Markdown,
                    data: json!({ "text": "# Title" }),
                }),
                SurfaceOp::ReplaceBlocks(Vec::new()),
                SurfaceOp::AppendText { block_id: BlockId::new("b-1"), text: " more".into() },
                SurfaceOp::SetActions(Vec::new()),
                SurfaceOp::SetStreaming(false),
            ],
        },
        DomainEvent::ObjectUpserted(Object {
            id: ObjectId::new("o-1"),
            name: "report.pdf".to_owned(),
            size: 2048,
            direction: Direction::Inbound,
            transfer: TransferState::Transferring { transferred: 1024 },
        }),
        DomainEvent::GrantRequested(grant("g-1", Risk::Critical)),
        DomainEvent::GrantResolved { grant_id: GrantId::new("g-1"), state: GrantState::Approved },
        DomainEvent::TaskUpserted(Task {
            id: TaskId::new("t-1"),
            intent_id: Some("i-1".to_owned()),
            context_id: ContextId::new("c-1"),
            actor: Actor::Agent(ParticipantId::new("p-1")),
            title: "Book travel".to_owned(),
            steps: vec![TaskStep { label: "Search".to_owned(), lifecycle: Lifecycle::Done }],
            lifecycle: Lifecycle::InProgress,
        }),
        DomainEvent::TaskStepChanged {
            task_id: TaskId::new("t-1"),
            index: 0,
            lifecycle: Lifecycle::NeedsMe,
        },
        DomainEvent::SourceStatusChanged(SourceStatus::Locked),
    ];
    for (i, kind) in all_moment_kinds().into_iter().enumerate() {
        let id = format!("m-{i}");
        events.push(added(moment(&id, "c-1", 100, kind, Lifecycle::NeedsMe)));
    }
    events
}

fn all_intent_kinds() -> Vec<IntentKind> {
    vec![
        IntentKind::SendMessage { context_id: ContextId::new("c-1"), text: "hi".into() },
        IntentKind::AttachObject { context_id: ContextId::new("c-1"), path: "/tmp/a.txt".into() },
        IntentKind::SurfaceAction {
            moment_id: MomentId::new("m-1"),
            surface_id: SurfaceId::new("s-1"),
            action_id: "run".into(),
            values: json!({ "count": 2 }),
            confirmed: false,
        },
        IntentKind::Approve { grant_id: GrantId::new("g-1"), confirmed: true },
        IntentKind::Deny { grant_id: GrantId::new("g-1") },
        IntentKind::Reveal { moment_id: MomentId::new("m-1") },
        IntentKind::ResolveConflict {
            moment_id: MomentId::new("m-1"),
            resolution: ConflictResolution::KeepLocal,
        },
        IntentKind::StartTask { context_id: ContextId::new("c-1"), request: "Plan a trip".into() },
        IntentKind::CancelTask { task_id: TaskId::new("t-1") },
        IntentKind::PinContext { context_id: ContextId::new("c-1") },
        IntentKind::UnpinContext { context_id: ContextId::new("c-1") },
        IntentKind::AcceptContact { moment_id: MomentId::new("m-1") },
        IntentKind::RejectContact { moment_id: MomentId::new("m-1") },
        IntentKind::VerifySafetyNumber { participant_id: ParticipantId::new("p-1") },
        IntentKind::Governance {
            context_id: ContextId::new("c-1"),
            op: GovernanceOp::SetRole {
                participant_id: ParticipantId::new("p-1"),
                role: Role::Admin,
            },
            confirmed: false,
        },
        IntentKind::AcceptHandoff { moment_id: MomentId::new("m-1") },
        IntentKind::DeclineHandoff { moment_id: MomentId::new("m-1") },
    ]
}

fn all_ui_event_kinds() -> Vec<UiEventKind> {
    vec![
        UiEventKind::SubmitMessage { text: "hi".into() },
        UiEventKind::AttachFile { path: "/tmp/a.txt".into() },
        UiEventKind::ActionInvoked { action_id: "run".into(), values: json!(null) },
        UiEventKind::ApprovePressed { confirmed: false },
        UiEventKind::DenyPressed,
        UiEventKind::RevealPressed,
        UiEventKind::ConflictResolved { resolution: ConflictResolution::KeepRemote },
        UiEventKind::TaskRequested { request: "Summarize".into() },
        UiEventKind::CancelPressed,
        UiEventKind::PinToggled { pinned: true },
        UiEventKind::ContactAnswered { accept: false },
        UiEventKind::ExpandToggled { expanded: true },
    ]
}

#[test]
fn every_domain_event_variant_round_trips_in_an_envelope() {
    let events = all_domain_events();
    let names: BTreeSet<_> = events.iter().map(domain_event_name).collect();
    assert_eq!(names.len(), DOMAIN_EVENT_VARIANTS, "a DomainEvent variant has no sample");
    for (seq, event) in events.into_iter().enumerate() {
        let env = EventEnvelope::new(seq as u64 + 1, Timestamp(1_000), event);
        let text = serde_json::to_string(&env).expect("serialize envelope");
        let back: EventEnvelope = serde_json::from_str(&text).expect("deserialize envelope");
        assert_eq!(back, env);
        assert_eq!(back.schema, EVENT_SCHEMA);
    }
}

#[test]
fn every_moment_kind_variant_round_trips() {
    let kinds = all_moment_kinds();
    let names: BTreeSet<_> = kinds.iter().map(moment_kind_name).collect();
    assert_eq!(names.len(), MOMENT_KIND_VARIANTS, "a MomentKind variant has no sample");
    for kind in kinds {
        let text = serde_json::to_string(&kind).expect("serialize kind");
        let back: MomentKind = serde_json::from_str(&text).expect("deserialize kind");
        assert_eq!(back, kind);
    }
}

#[test]
fn every_serializable_intent_kind_round_trips() {
    let kinds = all_intent_kinds();
    let names: BTreeSet<_> = kinds.iter().map(intent_kind_name).collect();
    assert_eq!(names.len(), SERIALIZABLE_INTENT_VARIANTS, "an IntentKind variant has no sample");
    for kind in kinds {
        let intent = Intent { actor: Actor::Agent(ParticipantId::new("p-1")), kind };
        let first = serde_json::to_value(&intent).expect("serialize intent");
        let back: Intent = serde_json::from_value(first.clone()).expect("deserialize intent");
        let second = serde_json::to_value(&back).expect("re-serialize intent");
        assert_eq!(first, second);
        assert_eq!(back.actor, intent.actor);
    }
}

#[test]
fn unlock_intent_refuses_to_serialize() {
    let intent = Intent::human(IntentKind::Unlock {
        account_id: AccountId::new("a-1"),
        secret: SecretInput::new("correct horse"),
    });
    assert_eq!(intent_kind_name(&intent.kind), "Unlock");
    let result = serde_json::to_string(&intent);
    assert!(result.is_err(), "Unlock must not serialize");
    let parsed = serde_json::from_str::<IntentKind>(r#"{"Unlock":{"account_id":"a-1"}}"#);
    assert!(parsed.is_err(), "Unlock must not deserialize");
}

#[test]
fn every_ui_event_kind_round_trips_with_scope_path() {
    let kinds = all_ui_event_kinds();
    let names: BTreeSet<_> = kinds.iter().map(ui_event_kind_name).collect();
    assert_eq!(names.len(), UI_EVENT_KIND_VARIANTS, "a UiEventKind variant has no sample");
    for kind in kinds {
        let event = UiEvent::new(Scope::Block(BlockId::new("b-1")), kind)
            .within(Scope::Surface(SurfaceId::new("s-1")))
            .within(Scope::Moment(MomentId::new("m-1")))
            .within(Scope::Context(ContextId::new("c-1")))
            .within(Scope::Workspace);
        let text = serde_json::to_string(&event).expect("serialize ui event");
        let back: UiEvent = serde_json::from_str(&text).expect("deserialize ui event");
        assert_eq!(back, event);
    }
}

#[test]
fn unknown_block_kind_deserializes_to_fallback() {
    let block: Block = serde_json::from_value(json!({
        "id": "b-x",
        "kind": "HolographicChart",
        "data": { "series": [1, 2, 3] }
    }))
    .expect("unknown block kind must not fail");
    assert_eq!(block.kind, BlockKind::Fallback);
    assert_eq!(block.data["series"][2], 3);

    let known: BlockKind = serde_json::from_str("\"Approval\"").expect("known kind");
    assert_eq!(known, BlockKind::Approval);
}

#[test]
fn every_catalog_block_kind_round_trips() {
    let catalog = [
        BlockKind::Text,
        BlockKind::Markdown,
        BlockKind::Heading,
        BlockKind::KeyValue,
        BlockKind::Metric,
        BlockKind::List,
        BlockKind::TableFallback,
        BlockKind::Form,
        BlockKind::Input,
        BlockKind::Select,
        BlockKind::Progress,
        BlockKind::LogStream,
        BlockKind::Warning,
        BlockKind::Error,
        BlockKind::Success,
        BlockKind::Approval,
        BlockKind::ActionGroup,
        BlockKind::FilePreview,
        BlockKind::Fallback,
    ];
    for kind in catalog {
        let text = serde_json::to_string(&kind).expect("serialize block kind");
        let back: BlockKind = serde_json::from_str(&text).expect("deserialize block kind");
        assert_eq!(back, kind);
    }
}
