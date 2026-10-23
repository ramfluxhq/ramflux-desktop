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
        DomainEvent::ContextSettingsChanged { .. } => "ContextSettingsChanged",
        DomainEvent::AccountLockChanged { .. } => "AccountLockChanged",
        DomainEvent::NodeSettingsChanged { .. } => "NodeSettingsChanged",
        DomainEvent::NodeProbed { .. } => "NodeProbed",
        DomainEvent::NodeRemoved { .. } => "NodeRemoved",
        DomainEvent::DeviceUpserted(_) => "DeviceUpserted",
        DomainEvent::GrantRevoked { .. } => "GrantRevoked",
        DomainEvent::StandingApprovalUpserted(_) => "StandingApprovalUpserted",
        DomainEvent::AuditAppended(_) => "AuditAppended",
        DomainEvent::IntegrationsUpdated(_) => "IntegrationsUpdated",
        DomainEvent::CapabilitiesUpdated(_) => "CapabilitiesUpdated",
        DomainEvent::DaemonInfoUpdated(_) => "DaemonInfoUpdated",
        DomainEvent::EffectFailed(_) => "EffectFailed",
        DomainEvent::ListingsLoaded(_) => "ListingsLoaded",
    }
}
const DOMAIN_EVENT_VARIANTS: usize = 29;

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
        IntentKind::MuteContext { .. } => "MuteContext",
        IntentKind::SetDisappearing { .. } => "SetDisappearing",
        IntentKind::PullObject { .. } => "PullObject",
        IntentKind::SaveObject { .. } => "SaveObject",
        IntentKind::RetryTransfer { .. } => "RetryTransfer",
        IntentKind::RevokeGrant { .. } => "RevokeGrant",
        IntentKind::RevokeStandingApproval { .. } => "RevokeStandingApproval",
        IntentKind::AddNode { .. } => "AddNode",
        IntentKind::ConfirmNode { .. } => "ConfirmNode",
        IntentKind::LockAccount { .. } => "LockAccount",
        IntentKind::RevokeDevice { .. } => "RevokeDevice",
        IntentKind::SyncDevices { .. } => "SyncDevices",
        IntentKind::ProbeNode { .. } => "ProbeNode",
        IntentKind::RemoveNode { .. } => "RemoveNode",
        IntentKind::SetDefaultNode { .. } => "SetDefaultNode",
        IntentKind::SetNodeEnabled { .. } => "SetNodeEnabled",
        IntentKind::ExportDiagnostics { .. } => "ExportDiagnostics",
        IntentKind::Unlock { .. }
        | IntentKind::CreateAccount { .. }
        | IntentKind::ActivateDevice { .. }
        | IntentKind::RestoreBackup { .. }
        | IntentKind::ExportBackup { .. }
        | IntentKind::RotatePassphrase { .. } => "secret",
    }
}
/// Serializable intent variants (all but the six secret-carrying kinds).
const SERIALIZABLE_INTENT_VARIANTS: usize = 34;

fn ui_event_kind_name(kind: &UiEventKind) -> &'static str {
    match kind {
        UiEventKind::SubmitMessage { .. } => "SubmitMessage",
        UiEventKind::AttachFile { .. } => "AttachFile",
        UiEventKind::ActionInvoked { .. } => "ActionInvoked",
        UiEventKind::ApprovePressed => "ApprovePressed",
        UiEventKind::DenyPressed => "DenyPressed",
        UiEventKind::RevealPressed => "RevealPressed",
        UiEventKind::ConflictResolved { .. } => "ConflictResolved",
        UiEventKind::TaskRequested { .. } => "TaskRequested",
        UiEventKind::CancelPressed => "CancelPressed",
        UiEventKind::PinToggled { .. } => "PinToggled",
        UiEventKind::ContactAnswered { .. } => "ContactAnswered",
        UiEventKind::ExpandToggled { .. } => "ExpandToggled",
        UiEventKind::MuteToggled { .. } => "MuteToggled",
        UiEventKind::DisappearingChosen { .. } => "DisappearingChosen",
        UiEventKind::VerifySafetyPressed { .. } => "VerifySafetyPressed",
        UiEventKind::HandoffAnswered { .. } => "HandoffAnswered",
        UiEventKind::ObjectActionPressed { .. } => "ObjectActionPressed",
        UiEventKind::GovernanceRequested { .. } => "GovernanceRequested",
        UiEventKind::RevokeGrantPressed { .. } => "RevokeGrantPressed",
        UiEventKind::RevokeStandingPressed { .. } => "RevokeStandingPressed",
        UiEventKind::AddNodeRequested { .. } => "AddNodeRequested",
        UiEventKind::ConfirmNodePressed { .. } => "ConfirmNodePressed",
        UiEventKind::NodeActionPressed { .. } => "NodeActionPressed",
        UiEventKind::LockPressed { .. } => "LockPressed",
        UiEventKind::RevokeDevicePressed { .. } => "RevokeDevicePressed",
        UiEventKind::DeviceSyncRequested { .. } => "DeviceSyncRequested",
        UiEventKind::DiagnosticsExportRequested { .. } => "DiagnosticsExportRequested",
        UiEventKind::FieldChanged { .. } => "FieldChanged",
        UiEventKind::FocusMomentRequested { .. } => "FocusMomentRequested",
    }
}
const UI_EVENT_KIND_VARIANTS: usize = 29;

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
            local: ConflictSide {
                version: "v3".to_owned(),
                summary: "Local edit".to_owned(),
                lines: vec!["groceries 120".to_owned()],
            },
            remote: ConflictSide {
                version: "v4".to_owned(),
                summary: "Remote edit".to_owned(),
                lines: Vec::new(),
            },
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
            kind: AccountKind::HeadlessAi,
            node_id: Some(NodeId::new("n-1")),
            device_id: Some("dev-1".to_owned()),
            lock_state: LockState::Unlocked,
            self_participant_id: Some(ParticipantId::new("p-me")),
        }),
        DomainEvent::NodeUpserted(Node {
            id: NodeId::new("n-1"),
            endpoints: vec!["quic://node.example.test:7443".to_owned()],
            fingerprint: Some("SHA256:0011".to_owned()),
            pin_source: PinSource::Invite,
            health: NodeHealth::Healthy,
            capabilities: vec!["relay".to_owned()],
            expires_at: Some(Timestamp(99)),
            confirmed: false,
            builtin: false,
            enabled: true,
            is_default: false,
            probe: None,
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
            saved_to: None,
            preview: Some("Quarterly report".to_owned()),
        }),
        DomainEvent::GrantRequested(grant("g-1", Risk::Critical)),
        DomainEvent::GrantResolved { grant_id: GrantId::new("g-1"), state: GrantState::Approved },
        DomainEvent::TaskUpserted(Task {
            id: TaskId::new("t-1"),
            intent_id: Some("i-1".to_owned()),
            context_id: ContextId::new("c-1"),
            actor: Actor::Agent(ParticipantId::new("p-1")),
            title: "Book travel".to_owned(),
            steps: vec![TaskStep {
                label: "Search".to_owned(),
                lifecycle: Lifecycle::Done,
                grant_id: Some(GrantId::new("g-1")),
            }],
            lifecycle: Lifecycle::InProgress,
            result: None,
        }),
        DomainEvent::TaskStepChanged {
            task_id: TaskId::new("t-1"),
            index: 0,
            lifecycle: Lifecycle::NeedsMe,
        },
        DomainEvent::SourceStatusChanged(SourceStatus::Locked),
        DomainEvent::ContextSettingsChanged {
            context_id: ContextId::new("c-1"),
            pinned: None,
            muted: Some(true),
            disappearing: Some(Disappearing::After(3_600)),
        },
        DomainEvent::AccountLockChanged {
            account_id: AccountId::new("a-1"),
            lock_state: LockState::Locked,
        },
        DomainEvent::NodeSettingsChanged {
            node_id: NodeId::new("n-1"),
            enabled: Some(false),
            is_default: None,
        },
        DomainEvent::NodeProbed {
            node_id: NodeId::new("n-1"),
            probe: NodeProbe {
                at: Timestamp(5),
                gateway: ProbeOutcome { ok: true, latency_ms: Some(18), detail: String::new() },
                relay: ProbeOutcome { ok: false, latency_ms: None, detail: "timeout".to_owned() },
            },
        },
        DomainEvent::NodeRemoved { node_id: NodeId::new("n-2") },
        DomainEvent::DeviceUpserted(Device {
            id: DeviceId::new("dev-1"),
            account_id: AccountId::new("a-1"),
            label: "Laptop".to_owned(),
            this_device: true,
            revoked: false,
            last_seen: Some(Timestamp(4)),
        }),
        DomainEvent::GrantRevoked { grant_id: GrantId::new("g-1") },
        DomainEvent::StandingApprovalUpserted(StandingApproval {
            id: StandingApprovalId::new("sa-1"),
            scope: "tool.calendar.read".to_owned(),
            max_risk: Risk::Medium,
            expiry: None,
            uses: 3,
            revoked: false,
        }),
        DomainEvent::AuditAppended(AuditEntry {
            at: Timestamp(6),
            action: "privacy.reveal".to_owned(),
            subject: "m-1".to_owned(),
            proposed_by: Actor::Human,
            human_confirmed: true,
        }),
        DomainEvent::IntegrationsUpdated(Integrations {
            mcp_servers: vec![McpServer {
                name: "files".to_owned(),
                tools: vec!["read".to_owned()],
                healthy: true,
            }],
            a2i_pending: vec![A2iItem { id: "a2i-1".to_owned(), summary: "Pause".to_owned() }],
        }),
        DomainEvent::CapabilitiesUpdated(Capabilities {
            unsupported: [("task.start".to_owned(), "K3".to_owned())].into_iter().collect(),
        }),
        DomainEvent::DaemonInfoUpdated(DaemonInfo {
            socket_path: "/tmp/rfd.sock".to_owned(),
            open_accounts: 1,
        }),
        DomainEvent::ListingsLoaded(Listing::ALL.to_vec()),
        DomainEvent::EffectFailed(EffectFailure {
            method: "grant.approve".to_owned(),
            code: FailureCode::Daemon("ValidationFailed".to_owned()),
            message: "approval expired".to_owned(),
        }),
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
        },
        IntentKind::Approve { grant_id: GrantId::new("g-1") },
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
        },
        IntentKind::AcceptHandoff { moment_id: MomentId::new("m-1") },
        IntentKind::DeclineHandoff { moment_id: MomentId::new("m-1") },
        IntentKind::MuteContext { context_id: ContextId::new("c-1"), muted: true },
        IntentKind::SetDisappearing { context_id: ContextId::new("c-1"), seconds: Some(60) },
        IntentKind::PullObject { object_id: ObjectId::new("o-1") },
        IntentKind::SaveObject { object_id: ObjectId::new("o-1"), path: "/tmp/o".into() },
        IntentKind::RetryTransfer { object_id: ObjectId::new("o-1") },
        IntentKind::RevokeGrant { grant_id: GrantId::new("g-1") },
        IntentKind::RevokeStandingApproval { approval_id: StandingApprovalId::new("sa-1") },
        IntentKind::AddNode { url: "https://node.example.test".into() },
        IntentKind::ConfirmNode { node_id: NodeId::new("n-1"), pin_source: PinSource::Manual },
        IntentKind::LockAccount { account_id: AccountId::new("a-1") },
        IntentKind::RevokeDevice { device_id: DeviceId::new("dev-2") },
        IntentKind::SyncDevices { direction: SyncDirection::Export, path: "/tmp/s".into() },
        IntentKind::ProbeNode { node_id: NodeId::new("n-1") },
        IntentKind::RemoveNode { node_id: NodeId::new("n-1") },
        IntentKind::SetDefaultNode { node_id: NodeId::new("n-1") },
        IntentKind::SetNodeEnabled { node_id: NodeId::new("n-1"), enabled: false },
        IntentKind::ExportDiagnostics { path: "/tmp/d".into() },
    ]
}

fn all_ui_event_kinds() -> Vec<UiEventKind> {
    vec![
        UiEventKind::SubmitMessage { text: "hi".into() },
        UiEventKind::AttachFile { path: "/tmp/a.txt".into() },
        UiEventKind::ActionInvoked { action_id: "run".into(), values: json!(null) },
        UiEventKind::ApprovePressed,
        UiEventKind::DenyPressed,
        UiEventKind::RevealPressed,
        UiEventKind::ConflictResolved { resolution: ConflictResolution::KeepRemote },
        UiEventKind::TaskRequested { request: "Summarize".into() },
        UiEventKind::CancelPressed,
        UiEventKind::PinToggled { pinned: true },
        UiEventKind::ContactAnswered { accept: false },
        UiEventKind::ExpandToggled { expanded: true },
        UiEventKind::MuteToggled { muted: true },
        UiEventKind::DisappearingChosen { seconds: None },
        UiEventKind::VerifySafetyPressed { participant_id: ParticipantId::new("p-1") },
        UiEventKind::HandoffAnswered { accept: true },
        UiEventKind::ObjectActionPressed {
            object_id: ObjectId::new("o-1"),
            action: ObjectAction::Save { path: "/tmp/o".into() },
        },
        UiEventKind::GovernanceRequested {
            op: GovernanceOp::Kick { participant_id: ParticipantId::new("p-2") },
        },
        UiEventKind::RevokeGrantPressed { grant_id: GrantId::new("g-1") },
        UiEventKind::RevokeStandingPressed { approval_id: StandingApprovalId::new("sa-1") },
        UiEventKind::AddNodeRequested { url: "https://node.example.test".into() },
        UiEventKind::ConfirmNodePressed { node_id: NodeId::new("n-1") },
        UiEventKind::NodeActionPressed {
            node_id: NodeId::new("n-1"),
            action: NodeAction::SetEnabled(false),
        },
        UiEventKind::LockPressed { account_id: AccountId::new("a-1") },
        UiEventKind::RevokeDevicePressed { device_id: DeviceId::new("dev-2") },
        UiEventKind::DeviceSyncRequested {
            direction: SyncDirection::Import,
            path: "/tmp/s".into(),
        },
        UiEventKind::DiagnosticsExportRequested { path: "/tmp/d".into() },
        UiEventKind::FieldChanged { value: "x".into() },
        UiEventKind::FocusMomentRequested { moment_id: "m-1".into() },
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
        let intent = Intent::agent(ParticipantId::new("p-1"), kind)
            .confirmed_by_human(ConfirmationIssuer::new().issue_hold(1_250).expect("a full hold"));
        let first = serde_json::to_value(&intent).expect("serialize intent");
        let back: Intent = serde_json::from_value(first.clone()).expect("deserialize intent");
        let mut second = serde_json::to_value(&back).expect("re-serialize intent");
        second["confirmed_by"] = first["confirmed_by"].clone();
        assert_eq!(first, second, "everything but the confirmation round-trips");
        assert_eq!(back.proposed_by, intent.proposed_by);
        assert!(intent.confirmed_by.is_some());
        assert!(back.confirmed_by.is_none(), "a deserialized intent is never confirmed");
        assert_eq!(first["confirmed_by"]["gesture"], "Hold");
    }
}

#[test]
fn secret_intents_refuse_to_serialize() {
    let secret = || SecretInput::new("correct horse");
    let kinds = [
        IntentKind::Unlock { account_id: AccountId::new("a-1"), secret: secret() },
        IntentKind::CreateAccount {
            kind: AccountKind::HumanInteractive,
            label: "Me".into(),
            node_id: NodeId::new("n-1"),
            secret: secret(),
        },
        IntentKind::ActivateDevice {
            node_id: NodeId::new("n-1"),
            device_label: "Laptop".into(),
            code: secret(),
        },
        IntentKind::RestoreBackup { path: "/tmp/b".into(), secret: secret() },
        IntentKind::ExportBackup {
            account_id: AccountId::new("a-1"),
            path: "/tmp/b".into(),
            secret: secret(),
        },
        IntentKind::RotatePassphrase {
            account_id: AccountId::new("a-1"),
            current: secret(),
            next: secret(),
        },
    ];
    for kind in kinds {
        assert_eq!(intent_kind_name(&kind), "secret");
        let intent = Intent::human(kind);
        assert!(serde_json::to_string(&intent).is_err(), "secret intents must not serialize");
    }
    for name in [
        "Unlock",
        "CreateAccount",
        "ActivateDevice",
        "RestoreBackup",
        "ExportBackup",
        "RotatePassphrase",
    ] {
        let text = format!(r#"{{"{name}":{{"account_id":"a-1"}}}}"#);
        assert!(serde_json::from_str::<IntentKind>(&text).is_err(), "{name} must not deserialize");
    }
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
