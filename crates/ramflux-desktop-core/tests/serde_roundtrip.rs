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
        DomainEvent::EffectSucceeded { .. } => "EffectSucceeded",
        DomainEvent::GroupInvitationsListed(_) => "GroupInvitationsListed",
        DomainEvent::MomentRemoved { .. } => "MomentRemoved",
        DomainEvent::GroupCreation { .. } => "GroupCreation",
        DomainEvent::UsernameStatusUpdated(_) => "UsernameStatusUpdated",
        DomainEvent::UsernameLookupUpdated { .. } => "UsernameLookupUpdated",
        DomainEvent::UsernameClaimUpdated { .. } => "UsernameClaimUpdated",
        DomainEvent::UsernameClaimSettled { .. } => "UsernameClaimSettled",
        DomainEvent::NodeUpserted(_) => "NodeUpserted",
        DomainEvent::NodeHealthChanged { .. } => "NodeHealthChanged",
        DomainEvent::ParticipantUpserted(_) => "ParticipantUpserted",
        DomainEvent::ContextUpserted(_) => "ContextUpserted",
        DomainEvent::ContextRemoved { .. } => "ContextRemoved",
        DomainEvent::MomentAdded(_) => "MomentAdded",
        DomainEvent::MomentUpdated { .. } => "MomentUpdated",
        DomainEvent::Surface(_) => "Surface",
        DomainEvent::Card(_) => "Card",
        DomainEvent::CardRuntimeUnavailable => "CardRuntimeUnavailable",
        DomainEvent::ObjectUpserted(_) => "ObjectUpserted",
        DomainEvent::GrantRequested(_) => "GrantRequested",
        DomainEvent::GrantResolved { .. } => "GrantResolved",
        DomainEvent::TaskUpserted(_) => "TaskUpserted",
        DomainEvent::TaskStepChanged { .. } => "TaskStepChanged",
        DomainEvent::TaskStateChanged { .. } => "TaskStateChanged",
        DomainEvent::TaskSnapshot(_) => "TaskSnapshot",
        DomainEvent::TaskAnswered { .. } => "TaskAnswered",
        DomainEvent::TaskRemoved { .. } => "TaskRemoved",
        DomainEvent::SourceStatusChanged(_) => "SourceStatusChanged",
        DomainEvent::ContextSettingsChanged { .. } => "ContextSettingsChanged",
        DomainEvent::AccountLockChanged { .. } => "AccountLockChanged",
        DomainEvent::AppLockChanged { .. } => "AppLockChanged",
        DomainEvent::AppLockCodeSheet { .. } => "AppLockCodeSheet",
        DomainEvent::AppLockCodeRefused { .. } => "AppLockCodeRefused",
        DomainEvent::AppLockLifecycleAnswered { .. } => "AppLockLifecycleAnswered",
        DomainEvent::NodeSettingsChanged { .. } => "NodeSettingsChanged",
        DomainEvent::NodeProbed { .. } => "NodeProbed",
        DomainEvent::NodeRemoved { .. } => "NodeRemoved",
        DomainEvent::DeviceUpserted(_) => "DeviceUpserted",
        DomainEvent::GrantRevoked { .. } => "GrantRevoked",
        DomainEvent::GrantFailed { .. } => "GrantFailed",
        DomainEvent::StandingApprovalUpserted(_) => "StandingApprovalUpserted",
        DomainEvent::AuditAppended(_) => "AuditAppended",
        DomainEvent::IntegrationsUpdated(_) => "IntegrationsUpdated",
        DomainEvent::CapabilitiesUpdated(_) => "CapabilitiesUpdated",
        DomainEvent::DaemonInfoUpdated(_) => "DaemonInfoUpdated",
        DomainEvent::SyncStatusChanged(_) => "SyncStatusChanged",
        DomainEvent::EffectFailed(_) => "EffectFailed",
        DomainEvent::AccountNetworkingFailed { .. } => "AccountNetworkingFailed",
        DomainEvent::SchemaGateChanged { .. } => "SchemaGateChanged",
        DomainEvent::ListingsLoaded(_) => "ListingsLoaded",
        DomainEvent::OwnContactCardUpdated(_) => "OwnContactCardUpdated",
        DomainEvent::CallUpserted(_) => "CallUpserted",
        DomainEvent::ContactRequestsListed(_) => "ContactRequestsListed",
        DomainEvent::ContactRequestUpserted(_) => "ContactRequestUpserted",
        DomainEvent::ContactRequestPolicyUpdated(_) => "ContactRequestPolicyUpdated",
        DomainEvent::ContactRequestFloorsUpdated(_) => "ContactRequestFloorsUpdated",
        DomainEvent::ContactRequestsReadFailed(_) => "ContactRequestsReadFailed",
        DomainEvent::BackupJobUpdated(_) => "BackupJobUpdated",
        DomainEvent::CeremonyUpdated(_) => "CeremonyUpdated",
        DomainEvent::RecoveryUpdated(_) => "RecoveryUpdated",
        DomainEvent::AgentsListed { .. } => "AgentsListed",
        DomainEvent::AgentRecordUpdated(_) => "AgentRecordUpdated",
        DomainEvent::AgentRosterStateChanged(_) => "AgentRosterStateChanged",
        DomainEvent::AgentCreationUpdated(_) => "AgentCreationUpdated",
        DomainEvent::AgentHostUpdated(_) => "AgentHostUpdated",
        DomainEvent::AgentCursorReset => "AgentCursorReset",
        DomainEvent::AgentJobsUpdated { .. } => "AgentJobsUpdated",
        DomainEvent::AgentApprovalsUpdated(_) => "AgentApprovalsUpdated",
    }
}
const DOMAIN_EVENT_VARIANTS: usize = 64;

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
        MomentKind::TaskEvent(_) => "TaskEvent",
        MomentKind::Call(_) => "Call",
        MomentKind::NameChange(_) => "NameChange",
        MomentKind::GroupRename(_) => "GroupRename",
    }
}
const MOMENT_KIND_VARIANTS: usize = 17;

fn intent_kind_name(kind: &IntentKind) -> &'static str {
    match kind {
        IntentKind::RetryMessage { .. } => "RetryMessage",
        IntentKind::SendMessage { .. } => "SendMessage",
        IntentKind::ReplyMessage { .. } => "ReplyMessage",
        IntentKind::EditMessage { .. } => "EditMessage",
        IntentKind::DeleteMessage { .. } => "DeleteMessage",
        IntentKind::LeaveGroup { .. } => "LeaveGroup",
        IntentKind::AnswerGroupInvitation { .. } => "AnswerGroupInvitation",
        IntentKind::AddContactByUsername { .. } => "AddContactByUsername",
        IntentKind::ResolveUsername { .. } => "ResolveUsername",
        IntentKind::UsernameStatusRefresh { .. } => "UsernameStatusRefresh",
        IntentKind::StartConversation { .. } => "StartConversation",
        IntentKind::AttachObject { .. } => "AttachObject",
        IntentKind::SurfaceAction { .. } => "SurfaceAction",
        IntentKind::RequestSurfaceUpdate { .. } => "RequestSurfaceUpdate",
        IntentKind::CardOpen { .. } => "CardOpen",
        IntentKind::CardPresent { .. } => "CardPresent",
        IntentKind::CardInteract { .. } => "CardInteract",
        IntentKind::CardSlotAnswer { .. } => "CardSlotAnswer",
        IntentKind::CardSubmit { .. } => "CardSubmit",
        IntentKind::CardRefresh { .. } => "CardRefresh",
        IntentKind::CardSuspend { .. } => "CardSuspend",
        IntentKind::CardDispose { .. } => "CardDispose",
        IntentKind::CardSuspendAll => "CardSuspendAll",
        IntentKind::CardResumeAll => "CardResumeAll",
        IntentKind::CardRetainVisible { .. } => "CardRetainVisible",
        IntentKind::Approve { .. } => "Approve",
        IntentKind::Deny { .. } => "Deny",
        IntentKind::Reveal { .. } => "Reveal",
        IntentKind::ResolveConflict { .. } => "ResolveConflict",
        IntentKind::StartTask { .. } => "StartTask",
        IntentKind::CancelTask { .. } => "CancelTask",
        IntentKind::RetryTaskDecision { .. } => "RetryTaskDecision",
        IntentKind::AcceptTaskProposal { .. } => "AcceptTaskProposal",
        IntentKind::PinContext { .. } => "PinContext",
        IntentKind::UnpinContext { .. } => "UnpinContext",
        IntentKind::AcceptContact { .. } => "AcceptContact",
        IntentKind::RejectContact { .. } => "RejectContact",
        IntentKind::VerifySafetyNumber { .. } => "VerifySafetyNumber",
        IntentKind::AddContact { .. } => "AddContact",
        IntentKind::AddContactCard { .. } => "AddContactCard",
        IntentKind::ExportContactCard => "ExportContactCard",
        IntentKind::CreateGroup { .. } => "CreateGroup",
        IntentKind::RenameContext { .. } => "RenameContext",
        IntentKind::Contact { .. } => "Contact",
        IntentKind::Governance { .. } => "Governance",
        IntentKind::AcceptHandoff { .. } => "AcceptHandoff",
        IntentKind::DeclineHandoff { .. } => "DeclineHandoff",
        IntentKind::MuteContext { .. } => "MuteContext",
        IntentKind::SetDisappearing { .. } => "SetDisappearing",
        IntentKind::PullObject { .. } => "PullObject",
        IntentKind::SaveObject { .. } => "SaveObject",
        IntentKind::RetryTransfer { .. } => "RetryTransfer",
        IntentKind::CancelTransfer { .. } => "CancelTransfer",
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
        IntentKind::AnswerAppLockCode { .. } => "AnswerAppLockCode",
        IntentKind::ConfigureAppLock { .. } => "ConfigureAppLock",
        IntentKind::AppLockLifecycle { .. } => "AppLockLifecycle",
        IntentKind::StartCall { .. } => "StartCall",
        IntentKind::AnswerContactRequest { .. } => "AnswerContactRequest",
        IntentKind::SetContactRequestPolicy { .. } => "SetContactRequestPolicy",
        IntentKind::ClearRequestFloors => "ClearRequestFloors",
        IntentKind::AnswerCall { .. } => "AnswerCall",
        IntentKind::RejectCall { .. } => "RejectCall",
        IntentKind::HangUpCall { .. } => "HangUpCall",
        IntentKind::InstallRestore { .. } => "InstallRestore",
        IntentKind::CancelBackupJob { .. } => "CancelBackupJob",
        IntentKind::BeginCeremony { .. } => "BeginCeremony",
        IntentKind::ShowRecoveryPhrase => "ShowRecoveryPhrase",
        IntentKind::HideRecoveryPhrase => "HideRecoveryPhrase",
        IntentKind::CancelCeremony => "CancelCeremony",
        IntentKind::AgentCreate { .. } => "AgentCreate",
        IntentKind::AgentOperationRefresh { .. } => "AgentOperationRefresh",
        IntentKind::AgentPersonaEdit { .. } => "AgentPersonaEdit",
        IntentKind::AgentAccessSet { .. } => "AgentAccessSet",
        IntentKind::AgentDelete { .. } => "AgentDelete",
        IntentKind::AgentJob { .. } => "AgentJob",
        IntentKind::AgentJobsRefresh { .. } => "AgentJobsRefresh",
        IntentKind::AgentWatchRefresh => "AgentWatchRefresh",
        IntentKind::AgentApprovalDecide { .. } => "AgentApprovalDecide",
        IntentKind::AgentRuntime { .. } => "AgentRuntime",
        IntentKind::BeginRecovery { .. } => "BeginRecovery",
        IntentKind::CancelRecovery => "CancelRecovery",
        IntentKind::Unlock { .. }
        | IntentKind::RestoreBackup { .. }
        | IntentKind::ExportBackup { .. }
        | IntentKind::CreateBackup { .. }
        | IntentKind::VerifyRestore { .. }
        | IntentKind::ConfirmCeremony { .. }
        | IntentKind::ConfirmRecovery { .. }
        | IntentKind::RotatePassphrase { .. }
        | IntentKind::ClaimUsername { .. } => "secret",
    }
}
/// Serializable intent variants (all but the nine secret-carrying kinds).
const SERIALIZABLE_INTENT_VARIANTS: usize = 91;

fn ui_event_kind_name(kind: &UiEventKind) -> &'static str {
    match kind {
        UiEventKind::CopyMessagePressed => "CopyMessagePressed",
        UiEventKind::RetryMessagePressed => "RetryMessagePressed",
        UiEventKind::SubmitMessage { .. } => "SubmitMessage",
        UiEventKind::ReplyPressed => "ReplyPressed",
        UiEventKind::EditPressed => "EditPressed",
        UiEventKind::DeleteForMePressed => "DeleteForMePressed",
        UiEventKind::DeleteForEveryonePressed => "DeleteForEveryonePressed",
        UiEventKind::SubmitReply { .. } => "SubmitReply",
        UiEventKind::SubmitEdit { .. } => "SubmitEdit",
        UiEventKind::LeaveGroupPressed => "LeaveGroupPressed",
        UiEventKind::GroupInvitationAnswered { .. } => "GroupInvitationAnswered",
        UiEventKind::AddContactByUsernameRequested { .. } => "AddContactByUsernameRequested",
        UiEventKind::ResolveUsernameRequested { .. } => "ResolveUsernameRequested",
        UiEventKind::UsernameStatusRefreshRequested { .. } => "UsernameStatusRefreshRequested",
        UiEventKind::StartConversationRequested { .. } => "StartConversationRequested",
        UiEventKind::AttachFile { .. } => "AttachFile",
        UiEventKind::ActionInvoked { .. } => "ActionInvoked",
        UiEventKind::SurfaceUpdateRequested => "SurfaceUpdateRequested",
        UiEventKind::CardInteracted { .. } => "CardInteracted",
        UiEventKind::CardSlotAnswered { .. } => "CardSlotAnswered",
        UiEventKind::CardSubmitRetried => "CardSubmitRetried",
        UiEventKind::ApprovePressed => "ApprovePressed",
        UiEventKind::DenyPressed => "DenyPressed",
        UiEventKind::RevealPressed => "RevealPressed",
        UiEventKind::ConflictResolved { .. } => "ConflictResolved",
        UiEventKind::TaskRequested { .. } => "TaskRequested",
        UiEventKind::CancelPressed => "CancelPressed",
        UiEventKind::RetryDecisionPressed => "RetryDecisionPressed",
        UiEventKind::ProposalAccepted => "ProposalAccepted",
        UiEventKind::PinToggled { .. } => "PinToggled",
        UiEventKind::ContactAnswered { .. } => "ContactAnswered",
        UiEventKind::ExpandToggled { .. } => "ExpandToggled",
        UiEventKind::MuteToggled { .. } => "MuteToggled",
        UiEventKind::DisappearingChosen { .. } => "DisappearingChosen",
        UiEventKind::VerifySafetyPressed { .. } => "VerifySafetyPressed",
        UiEventKind::AddContactRequested { .. } => "AddContactRequested",
        UiEventKind::AddContactCardRequested { .. } => "AddContactCardRequested",
        UiEventKind::ShareContactCardRequested => "ShareContactCardRequested",
        UiEventKind::CreateGroupRequested { .. } => "CreateGroupRequested",
        UiEventKind::RenameRequested { .. } => "RenameRequested",
        UiEventKind::ContactActionPressed { .. } => "ContactActionPressed",
        UiEventKind::HandoffAnswered { .. } => "HandoffAnswered",
        UiEventKind::ObjectActionPressed { .. } => "ObjectActionPressed",
        UiEventKind::GovernanceRequested { .. } => "GovernanceRequested",
        UiEventKind::RevokeGrantPressed { .. } => "RevokeGrantPressed",
        UiEventKind::RevokeStandingPressed { .. } => "RevokeStandingPressed",
        UiEventKind::AgentApprovalPressed { .. } => "AgentApprovalPressed",
        UiEventKind::AddNodeRequested { .. } => "AddNodeRequested",
        UiEventKind::ConfirmNodePressed { .. } => "ConfirmNodePressed",
        UiEventKind::NodeActionPressed { .. } => "NodeActionPressed",
        UiEventKind::LockPressed { .. } => "LockPressed",
        UiEventKind::AppLockConfigureRequested { .. } => "AppLockConfigureRequested",
        UiEventKind::RevokeDevicePressed { .. } => "RevokeDevicePressed",
        UiEventKind::DeviceSyncRequested { .. } => "DeviceSyncRequested",
        UiEventKind::DiagnosticsExportRequested { .. } => "DiagnosticsExportRequested",
        UiEventKind::FieldChanged { .. } => "FieldChanged",
        UiEventKind::FocusMomentRequested { .. } => "FocusMomentRequested",
        UiEventKind::CallStartRequested => "CallStartRequested",
        UiEventKind::ContactRequestAnswered { .. } => "ContactRequestAnswered",
        UiEventKind::ContactRequestPolicySwitched { .. } => "ContactRequestPolicySwitched",
        UiEventKind::ClearRequestFloorsPressed => "ClearRequestFloorsPressed",
        UiEventKind::CallAnswered { .. } => "CallAnswered",
        UiEventKind::CallHangUpPressed { .. } => "CallHangUpPressed",
    }
}
const UI_EVENT_KIND_VARIANTS: usize = 63;

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
            unattributed: false,
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
            content: Some(Protected::new("1234")),
        }),
        MomentKind::Handoff(HandoffPayload {
            from_device: ParticipantId::new("d-laptop"),
            to_device: None,
            summary: "Continue the draft".to_owned(),
        }),
        MomentKind::Fallback(FallbackPayload {
            reason: "unsupported surface".to_owned(),
            text: "Open on a newer client".to_owned(),
            unsupported: None,
        }),
        MomentKind::System(SystemPayload { level: SystemLevel::Warning, text: "Node slow".into() }),
        MomentKind::TaskEvent(TaskEventPayload {
            task_id: TaskId::new("t-1"),
            event: TaskEventKind::Denied { step_id: Some(StepId::new("deploy")) },
            actor_kind_verified: true,
            actor_kind: Some(PeerKind::HeadlessAi),
        }),
        MomentKind::GroupRename(GroupRenamePayload {
            old_name: "Checklist".to_owned(),
            new_name: "Reviewed checklist".to_owned(),
        }),
        MomentKind::Call(CallPayload { call_id: CallId::new("k-1") }),
        MomentKind::NameChange(NameChangePayload {
            participant_id: ParticipantId::new("p-kai"),
            old_name: "Kai".to_owned(),
            new_name: "Kai Ortiz".to_owned(),
        }),
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
            secure_sessions: ramflux_desktop_core::SecureSessions::Reestablishing,
            networking_session: None,
            networking_custody: None,
            authority_blocked: false,
            networking_failure: None,
            networking_reason: None,
            networking: ramflux_desktop_core::Networking::Pending,
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
        DomainEvent::Card(CardEvent::Refused {
            surface_id: SurfaceId::new("s-1"),
            reason: "card_session_refused".into(),
        }),
        DomainEvent::CardRuntimeUnavailable,
        DomainEvent::ObjectUpserted(Object {
            mime: None,
            local_path: None,
            id: ObjectId::new("o-1"),
            name: Some("report.pdf".to_owned()),
            size: Some(2048),
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
            requester: Actor::Human,
            assignee: ParticipantId::new("p-1"),
            title: "Book travel".to_owned(),
            steps: vec![
                TaskStep {
                    id: StepId::new("search"),
                    label: "Search".to_owned(),
                    state: StepState::Done,
                    needs_me: Some(NeedsMe::Approval { grant_id: GrantId::new("g-1") }),
                },
                TaskStep {
                    id: StepId::new("pick"),
                    label: "Pick".to_owned(),
                    state: StepState::NeedsMe,
                    needs_me: Some(NeedsMe::Choice { surface_id: SurfaceId::new("s-pick") }),
                },
            ],
            state: TaskState::NeedsMe,
            version: 3,
            result: Some("Booked".to_owned()),
            result_ref: Some(SurfaceId::new("s-result")),
            flags: TaskFlags::default(),
            actors: TaskActors::default(),
            answer: None,
        }),
        DomainEvent::TaskStepChanged {
            task_id: TaskId::new("t-1"),
            version: 4,
            step_id: StepId::new("pick"),
            state: StepState::NeedsMe,
            needs_me: Some(NeedsMe::Input { surface_id: SurfaceId::new("s-form") }),
        },
        DomainEvent::TaskStateChanged {
            task_id: TaskId::new("t-1"),
            version: 5,
            state: TaskState::Cancelled,
            result: Some("Cancelled".into()),
        },
        DomainEvent::TaskSnapshot(Task {
            id: TaskId::new("t-2"),
            intent_id: None,
            context_id: ContextId::new("c-1"),
            requester: Actor::Member(ParticipantId::new("p-2")),
            assignee: ParticipantId::new("p-1"),
            title: "Check".to_owned(),
            steps: Vec::new(),
            state: TaskState::Running,
            version: 9,
            result: None,
            result_ref: None,
            flags: TaskFlags { stalled: true, compacted: true },
            actors: TaskActors {
                requester: None,
                assignee: Some(TaskActor {
                    participant_id: ParticipantId::new("p-orbit"),
                    device_id: Some(DeviceId::new("device-orbit")),
                    kind: Some(PeerKind::HeadlessAi),
                    kind_verified: true,
                }),
                acceptor: None,
            },
            answer: Some(TaskAnswer::Refused(TaskRefusal::Other("x".into()))),
        }),
        DomainEvent::TaskAnswered {
            task_id: TaskId::new("t-2"),
            answer: TaskAnswer::DecisionPending { step_id: StepId::new("s") },
        },
        DomainEvent::TaskAnswered {
            task_id: TaskId::new("t-2"),
            answer: TaskAnswer::DecisionUndelivered {
                step_id: StepId::new("s"),
                grant_id: GrantId::new("g-1"),
                approved: true,
            },
        },
        DomainEvent::TaskRemoved { task_id: TaskId::new("t-2") },
        DomainEvent::Surface(SurfaceEvent::Snapshot(Box::new(SurfaceSnapshot {
            moment_id: MomentId::new("m-s"),
            context_id: ContextId::new("c-1"),
            origin: ParticipantId::new("p-1"),
            at: Some(Timestamp(9)),
            surface_id: SurfaceId::new("s-1"),
            version: 7,
            hash: SurfaceHash::new("Qh7"),
            actions: Vec::new(),
            fallback_text: "A reply".into(),
            closed: false,
            out_of_sync: true,
            order: Some(OrderKey { hlc: 7, author: "a/d".into(), device_sequence: 2 }),
        }))),
        DomainEvent::Surface(SurfaceEvent::ActionRefused {
            surface_id: SurfaceId::new("s-1"),
            version: 6,
        }),
        DomainEvent::Surface(SurfaceEvent::UpdateRequested {
            surface_id: SurfaceId::new("s-1"),
            refused: Some("backend.surface.rate_limited".into()),
        }),
        DomainEvent::Surface(SurfaceEvent::UpdatedAt {
            surface_id: SurfaceId::new("s-1"),
            at: Timestamp(1_790_000_090_000),
        }),
        DomainEvent::Surface(SurfaceEvent::Removed { surface_id: SurfaceId::new("s-1") }),
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
        DomainEvent::AppLockChanged {
            account_id: AccountId::new("a-1"),
            lock: AppLock {
                enabled: true,
                timeout_ms: 60_000,
                state: AppLockState::Locked,
                reason: Some("timeout".into()),
                generation: 3,
                armed: false,
                code_retry_after_ms: None,
                code_attempts_exhausted: false,
            },
        },
        DomainEvent::AppLockLifecycleAnswered {
            account_id: AccountId::new("a-1"),
            phase: ramflux_desktop_core::AppLockPhase::Foreground,
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
        DomainEvent::GrantFailed {
            grant_id: GrantId::new("g-1"),
            failure: GrantFailure::ArgumentsMismatch,
        },
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
                healthy: Some(true),
            }],
            a2i_pending: vec![A2iItem { id: "a2i-1".to_owned(), summary: "Pause".to_owned() }],
        }),
        DomainEvent::CapabilitiesUpdated(Capabilities {
            unsupported: [("task.start".to_owned(), "K3".to_owned())].into_iter().collect(),
            born_v3_creation: true,
        }),
        DomainEvent::CeremonyUpdated(Ceremony {
            phase: CeremonyPhase::Challenged { positions: [3, 11, 20] },
            ceremony_id: "ceremony-1".to_owned(),
            label: "Alex".to_owned(),
            expires_at: Timestamp(300_000),
        }),
        DomainEvent::CeremonyUpdated(Ceremony {
            phase: CeremonyPhase::Created(CeremonyAccount {
                local_account_id: "a-1".to_owned(),
                display_name: "Alex".to_owned(),
                networking: Networking::Live,
            }),
            ceremony_id: "ceremony-1".to_owned(),
            label: "Alex".to_owned(),
            expires_at: Timestamp(300_000),
        }),
        DomainEvent::CeremonyUpdated(Ceremony {
            phase: CeremonyPhase::Failed(CeremonyFailure::ConfirmationFailed),
            ceremony_id: String::new(),
            label: "Alex".to_owned(),
            expires_at: Timestamp(0),
        }),
        DomainEvent::RecoveryUpdated(RecoveryCeremony {
            phase: RecoveryPhase::Prepared,
            account_id: AccountId::new("a-1"),
            anchor: "anchor-1".to_owned(),
            ceremony_id: "recovery-1".to_owned(),
            expires_at: Timestamp(300_000),
        }),
        DomainEvent::RecoveryUpdated(RecoveryCeremony {
            phase: RecoveryPhase::Recovered(CeremonyAccount {
                local_account_id: "a-1".to_owned(),
                display_name: "Alex".to_owned(),
                networking: Networking::Pending,
            }),
            account_id: AccountId::new("a-1"),
            anchor: "anchor-1".to_owned(),
            ceremony_id: "recovery-1".to_owned(),
            expires_at: Timestamp(300_000),
        }),
        DomainEvent::RecoveryUpdated(RecoveryCeremony {
            phase: RecoveryPhase::Failed(CeremonyFailure::ConfirmationFailed),
            account_id: AccountId::new("a-1"),
            anchor: String::new(),
            ceremony_id: String::new(),
            expires_at: Timestamp(0),
        }),
        DomainEvent::AgentsListed { records: vec![sample_agent()], journal_cursor: "c-1".into() },
        DomainEvent::AgentRecordUpdated(Box::new(sample_agent())),
        DomainEvent::AgentRosterStateChanged(AgentRosterState::Offline),
        DomainEvent::AgentCursorReset,
        DomainEvent::AgentCreationUpdated(sample_creation()),
        DomainEvent::AgentHostUpdated(AgentHostStatus {
            state: AgentHostState::Running,
            prx_version: Some("0.1.0".into()),
            refusal: None,
            state_dir: Some("/state/prx".into()),
            applied: true,
            nonce: 1,
        }),
        DomainEvent::AgentJobsUpdated {
            agent: "agent-anchor".into(),
            jobs: vec![AgentJob {
                id: "job-1".into(),
                kind: AgentJobKind::Cron,
                creator_agent: None,
            }],
        },
        DomainEvent::AgentApprovalsUpdated(vec![AgentApproval {
            approval_id: "ap-1".into(),
            conversation_id: "conv-1".into(),
            status: "pending".into(),
            agent: "agent-1".into(),
            tool: "shell".into(),
            preview: "ls".into(),
            expires_at: Some(1_900_000_000),
        }]),
        DomainEvent::DaemonInfoUpdated(DaemonInfo {
            socket_path: "/tmp/rfd.sock".to_owned(),
            open_accounts: 1,
        }),
        DomainEvent::SyncStatusChanged(SyncStatus {
            mode: SyncMode::Stream,
            reason: None,
            cursor: Some(7),
            daemon_sync: Some(DaemonSyncState::Running),
            last_run: Some(Timestamp(9)),
            unrouted: Some(1),
        }),
        DomainEvent::ListingsLoaded(Listing::ALL.to_vec()),
        DomainEvent::EffectSucceeded { method: "node.add".to_owned() },
        DomainEvent::MomentRemoved { moment_id: MomentId::new("m-1") },
        DomainEvent::GroupInvitationsListed(vec![GroupInvitation {
            group_id: "g-1".to_owned(),
            name: Some("Group".to_owned()),
            inviter: None,
            expires_at: Some(Timestamp(5)),
        }]),
        DomainEvent::GroupCreation { context_id: ContextId::new("c-1"), creating: true },
        DomainEvent::UsernameStatusUpdated(UsernameStatus::Unavailable("refused".to_owned())),
        DomainEvent::OwnContactCardUpdated(ContactCard {
            text: "card-text".to_owned(),
            link: Some("ramflux:contact/card".to_owned()),
        }),
        DomainEvent::SchemaGateChanged {
            account_id: AccountId::new("a"),
            gate: SchemaGate::UpdateApp,
            generation: Some(1),
        },
        DomainEvent::AccountNetworkingFailed {
            account_id: AccountId::new("a"),
            code: FailureCode::Daemon("CustodyForkFenced".into()),
        },
        DomainEvent::EffectFailed(EffectFailure {
            method: "grant.approve".to_owned(),
            code: FailureCode::Daemon("ValidationFailed".to_owned()),
            message: "approval expired".to_owned(),
        }),
        DomainEvent::CallUpserted(Call {
            id: CallId::new("k-1"),
            context_id: ContextId::new("c-1"),
            peer: ParticipantId::new("p-1"),
            caller: ParticipantId::new("p-1"),
            direction: CallDirection::Incoming,
            state: CallState::Ended(CallEndReason::Timeout),
            version: 2,
            started_at: Timestamp(10),
            answered_at: None,
            ended_at: Some(Timestamp(45_010)),
            order: Some(OrderKey { hlc: 9, author: "a/d".into(), device_sequence: 3 }),
        }),
        DomainEvent::ContactRequestsListed(vec![contact_request()]),
        DomainEvent::ContactRequestUpserted(ContactRequest {
            state: RequestState::Declined,
            direction: RequestDirection::Outgoing,
            ..contact_request()
        }),
        DomainEvent::ContactRequestPolicyUpdated(RequestPolicy::AutoAccept),
        DomainEvent::ContactRequestFloorsUpdated(ramflux_desktop_core::RequestFloors {
            retained: 4096,
            clearable: 12,
            limit: 4096,
            saturated: true,
        }),
        DomainEvent::ContactRequestsReadFailed("Unavailable".to_owned()),
        DomainEvent::BackupJobUpdated(Box::new(BackupJob {
            id: BackupJobId::new("j-1"),
            kind: BackupJobKind::Restore,
            path: "/tmp/b".to_owned(),
            phase: BackupPhase::Done,
            version: 7,
            started_at: Timestamp(10),
            finished_at: Some(Timestamp(90)),
            progress: BackupProgress {
                category: BackupCategory::Attachments,
                items_done: 3,
                items_total: Some(3),
                bytes_done: 30,
                bytes_total: Some(30),
            },
            summary: None,
            preview: Some(RestorePreview {
                account_id: AccountId::new("a-1"),
                account_label: Some("Me".to_owned()),
                created_at: Timestamp(5),
                counts: BackupCounts { messages: 2, attachments: 1, ..BackupCounts::default() },
                file_bytes: 30,
                merge: true,
                attachments: AttachmentCheck {
                    verified: 1,
                    missing: 1,
                    unavailable_at_source: 1,
                    ..AttachmentCheck::default()
                },
            }),
            report: Some(RestoreReport {
                restored: BackupCounts { messages: 1, ..BackupCounts::default() },
                kept_newer: BackupCounts { messages: 1, ..BackupCounts::default() },
                attachments: AttachmentCheck {
                    verified: 1,
                    mismatched: 0,
                    missing: 1,
                    unavailable_at_source: 1,
                    local_missing_kept: 2,
                },
            }),
            failure: Some(BackupFailure::InsufficientSpace { needed: Some(9), available: None }),
        })),
    ];
    for (i, kind) in all_moment_kinds().into_iter().enumerate() {
        let id = format!("m-{i}");
        events.push(added(moment(&id, "c-1", 100, kind, Lifecycle::NeedsMe)));
    }
    events
}

fn contact_request() -> ContactRequest {
    ContactRequest {
        id: "rq-1".to_owned(),
        peer: ParticipantId::new("p-1"),
        direction: RequestDirection::Incoming,
        state: RequestState::Pending,
        note: Some("from the meetup".to_owned()),
        safety_number: Some("12345 67890".to_owned()),
        held_messages: 2,
        preview: Some("hi there".to_owned()),
        expires_at: Some(Timestamp(604_800_000)),
    }
}

fn sample_persona() -> AgentPersona {
    AgentPersona {
        name: "Research helper".into(),
        avatar: "preset:spark:teal".into(),
        responsibility: "Reads papers".into(),
    }
}

fn sample_request() -> AgentCreateRequest {
    AgentCreateRequest {
        persona: sample_persona(),
        config: AgentConfig {
            provider: "anthropic".into(),
            model: "model-x".into(),
            autonomy: "full".into(),
            tools: AgentTools::None,
            prompt: None,
            max_concurrent_turns: 1,
            max_cost_per_day_cents: 100,
        },
        access: AgentAccess::owner_only(),
    }
}

fn sample_agent() -> AgentRecord {
    AgentRecord {
        agent_anchor: "agent-anchor".into(),
        custodian_anchor: "a-1".into(),
        persona: sample_persona(),
        lifecycle: AgentLifecycle::Active,
        access: AgentAccess::owner_only(),
        role: "named".into(),
        base: serde_json::json!({"id": "base-1"}),
        runtime_name: "a_name".into(),
    }
}

fn sample_creation() -> AgentCreation {
    AgentCreation {
        request_id: "req-1".into(),
        request_hash: "hash-1".into(),
        state: AgentCreationState::Failed(AgentRefusal::PrxSpawn),
        agent_anchor: None,
        expires_at: None,
    }
}

fn all_intent_kinds() -> Vec<IntentKind> {
    vec![
        IntentKind::AnswerContactRequest {
            request_id: "rq-1".to_owned(),
            answer: RequestAnswer::Block,
        },
        IntentKind::SetContactRequestPolicy { policy: RequestPolicy::Manual },
        IntentKind::ClearRequestFloors,
        IntentKind::RetryMessage { moment_id: MomentId::new("m-1") },
        IntentKind::SendMessage { context_id: ContextId::new("c-1"), text: "hi".into() },
        IntentKind::ReplyMessage { moment_id: MomentId::new("m-1"), text: "re".into() },
        IntentKind::EditMessage { moment_id: MomentId::new("m-1"), text: "ed".into() },
        IntentKind::DeleteMessage { moment_id: MomentId::new("m-1"), scope: DeleteScope::Everyone },
        IntentKind::LeaveGroup { context_id: ContextId::new("c-1") },
        IntentKind::AnswerGroupInvitation { group_id: "g-1".into(), accept: true },
        IntentKind::AddContactByUsername { name: "ana".into() },
        IntentKind::ResolveUsername { name: "@ana".into() },
        IntentKind::UsernameStatusRefresh { operation_id: Some("op-1".into()) },
        IntentKind::StartConversation {
            context_id: ContextId::new("c-new"),
            participant_id: ParticipantId::new("p-ana"),
            text: "hi".into(),
        },
        IntentKind::AttachObject { context_id: ContextId::new("c-1"), path: "/tmp/a.txt".into() },
        IntentKind::SurfaceAction {
            moment_id: MomentId::new("m-1"),
            surface_id: SurfaceId::new("s-1"),
            action_id: "run".into(),
            values: json!({ "count": 2 }),
            surface_hash: SurfaceHash::new("sha256:abc"),
            version: 2,
        },
        IntentKind::RequestSurfaceUpdate {
            moment_id: MomentId::new("m-1"),
            surface_id: SurfaceId::new("s-1"),
        },
        IntentKind::CardOpen {
            moment_id: MomentId::new("m-1"),
            surface_id: SurfaceId::new("s-1"),
            locale: "en".into(),
            dark: false,
        },
        IntentKind::CardInteract {
            moment_id: MomentId::new("m-1"),
            surface_id: SurfaceId::new("s-1"),
            key: "field".into(),
            event: CardInput::Input { value: json!("text") },
            seen: CardSeen::default(),
        },
        IntentKind::CardSlotAnswer {
            moment_id: MomentId::new("m-1"),
            surface_id: SurfaceId::new("s-1"),
            slot_id: "[link,link-1]".into(),
            outcome: CardSlotOutcome::Cancel,
            seen: CardSeen::default(),
        },
        IntentKind::CardSuspendAll,
        IntentKind::CardResumeAll,
        IntentKind::CardPresent {
            moment_id: MomentId::new("m-1"),
            surface_id: SurfaceId::new("s-1"),
            locale: "ar".into(),
            dark: true,
        },
        IntentKind::CardSubmit {
            moment_id: MomentId::new("m-1"),
            surface_id: SurfaceId::new("s-1"),
        },
        IntentKind::CardRefresh {
            moment_id: MomentId::new("m-1"),
            surface_id: SurfaceId::new("s-1"),
        },
        IntentKind::CardSuspend {
            moment_id: MomentId::new("m-1"),
            surface_id: SurfaceId::new("s-1"),
        },
        IntentKind::CardDispose {
            moment_id: MomentId::new("m-1"),
            surface_id: SurfaceId::new("s-1"),
        },
        IntentKind::CardRetainVisible { surface_ids: BTreeSet::from([SurfaceId::new("s-1")]) },
        IntentKind::Approve { grant_id: GrantId::new("g-1") },
        IntentKind::Deny { grant_id: GrantId::new("g-1") },
        IntentKind::Reveal { moment_id: MomentId::new("m-1") },
        IntentKind::ResolveConflict {
            moment_id: MomentId::new("m-1"),
            resolution: ConflictResolution::KeepLocal,
        },
        IntentKind::StartTask {
            context_id: ContextId::new("c-1"),
            assignee: ParticipantId::new("p-orbit"),
            text: "Plan a trip".into(),
        },
        IntentKind::AcceptTaskProposal { task_id: TaskId::new("t-1") },
        IntentKind::CancelTask { task_id: TaskId::new("t-1") },
        IntentKind::RetryTaskDecision { task_id: TaskId::new("t-1") },
        IntentKind::PinContext { context_id: ContextId::new("c-1") },
        IntentKind::UnpinContext { context_id: ContextId::new("c-1") },
        IntentKind::AcceptContact { moment_id: MomentId::new("m-1") },
        IntentKind::RejectContact { moment_id: MomentId::new("m-1") },
        IntentKind::VerifySafetyNumber { participant_id: ParticipantId::new("p-1") },
        IntentKind::AddContact { participant_id: ParticipantId::new("principal-ana") },
        IntentKind::AddContactCard {
            card: "ramflux:contact/card".to_owned(),
            replace_provisional_root: Some("root-old".to_owned()),
        },
        IntentKind::ExportContactCard,
        IntentKind::CreateGroup {
            context_id: ContextId::new("g-new"),
            title: "Team".to_owned(),
            members: vec![ParticipantId::new("p-1")],
        },
        IntentKind::RenameContext { context_id: ContextId::new("g-1"), title: "Crew".to_owned() },
        IntentKind::Contact {
            participant_id: ParticipantId::new("p-1"),
            action: ContactAction::Remove,
        },
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
        IntentKind::ConfigureAppLock {
            account_id: AccountId::new("a-1"),
            enabled: true,
            timeout_ms: 60_000,
        },
        IntentKind::AppLockLifecycle {
            account_id: AccountId::new("a-1"),
            phase: AppLockPhase::Background,
        },
        IntentKind::StartCall { context_id: "c-1".into() },
        IntentKind::AnswerCall { call_id: "k-1".into() },
        IntentKind::RejectCall { call_id: "k-1".into() },
        IntentKind::HangUpCall { call_id: "k-1".into() },
        IntentKind::InstallRestore { job_id: "j-1".into() },
        IntentKind::CancelBackupJob { job_id: "j-1".into() },
        IntentKind::BeginCeremony { label: "Alex".into() },
        IntentKind::ShowRecoveryPhrase,
        IntentKind::HideRecoveryPhrase,
        IntentKind::CancelCeremony,
        IntentKind::AgentCreate { owner_id: AccountId::new("a-1"), request: sample_request() },
        IntentKind::AgentOperationRefresh { request_id: "req-1".into() },
        IntentKind::AgentJobsRefresh { agent: "agent-anchor".into() },
        IntentKind::AgentWatchRefresh,
        IntentKind::AgentPersonaEdit { agent: "agent-anchor".into(), persona: sample_persona() },
        IntentKind::AgentAccessSet {
            agent: "agent-anchor".into(),
            access: AgentAccess::owner_only(),
        },
        IntentKind::AgentDelete { agent: "agent-anchor".into() },
        IntentKind::AgentJob {
            agent: "agent-anchor".into(),
            kind: AgentJobKind::XinTask,
            id: "job-1".into(),
            action: AgentJobAction::Pause,
        },
        IntentKind::AgentApprovalDecide {
            owner_id: AccountId::new("a-1"),
            approval_id: "ap-1".into(),
            approved: true,
        },
        IntentKind::AgentRuntime {
            action: AgentRuntimeAction::Install,
            provider: "anthropic".into(),
            model: "model-x".into(),
            key: Some(SecretInput::new("sk-test")),
            nonce: 1,
        },
        IntentKind::BeginRecovery { account_id: AccountId::new("a-1") },
        IntentKind::CancelRecovery,
    ]
}

fn all_ui_event_kinds() -> Vec<UiEventKind> {
    vec![
        UiEventKind::RetryMessagePressed,
        UiEventKind::CopyMessagePressed,
        UiEventKind::SubmitMessage { text: "hi".into() },
        UiEventKind::ReplyPressed,
        UiEventKind::EditPressed,
        UiEventKind::DeleteForMePressed,
        UiEventKind::DeleteForEveryonePressed,
        UiEventKind::SubmitReply { target: MomentId::new("m-1"), text: "re".into() },
        UiEventKind::SubmitEdit { target: MomentId::new("m-1"), text: "ed".into() },
        UiEventKind::LeaveGroupPressed,
        UiEventKind::GroupInvitationAnswered { group_id: "g-1".into(), accept: false },
        UiEventKind::AddContactByUsernameRequested { name: "@ana".into() },
        UiEventKind::ResolveUsernameRequested { name: "@ana".into() },
        UiEventKind::UsernameStatusRefreshRequested { operation_id: Some("op-1".into()) },
        UiEventKind::StartConversationRequested {
            context_id: ContextId::new("c-new"),
            participant_id: ParticipantId::new("p-ana"),
            text: "hi".into(),
        },
        UiEventKind::AttachFile { path: "/tmp/a.txt".into() },
        UiEventKind::ActionInvoked {
            action_id: "run".into(),
            values: json!(null),
            surface_hash: SurfaceHash::new("sha256:abc"),
            version: 2,
        },
        UiEventKind::SurfaceUpdateRequested,
        UiEventKind::CardInteracted {
            key: "field".into(),
            event: CardInput::Activate,
            seen: CardSeen::default(),
        },
        UiEventKind::CardSlotAnswered {
            slot_id: "[approval,root]".into(),
            outcome: CardSlotOutcome::Deny,
            seen: CardSeen::default(),
        },
        UiEventKind::CardSubmitRetried,
        UiEventKind::ApprovePressed,
        UiEventKind::DenyPressed,
        UiEventKind::RevealPressed,
        UiEventKind::ConflictResolved { resolution: ConflictResolution::KeepRemote },
        UiEventKind::TaskRequested {
            assignee: ParticipantId::new("p-orbit"),
            text: "Summarize".into(),
        },
        UiEventKind::ProposalAccepted,
        UiEventKind::CancelPressed,
        UiEventKind::RetryDecisionPressed,
        UiEventKind::PinToggled { pinned: true },
        UiEventKind::ContactAnswered { accept: false },
        UiEventKind::ExpandToggled { expanded: true },
        UiEventKind::MuteToggled { muted: true },
        UiEventKind::DisappearingChosen { seconds: None },
        UiEventKind::VerifySafetyPressed { participant_id: ParticipantId::new("p-1") },
        UiEventKind::AddContactRequested { participant_id: ParticipantId::new("principal-ana") },
        UiEventKind::AddContactCardRequested {
            card: "ramflux:contact/card".to_owned(),
            replace_provisional_root: None,
        },
        UiEventKind::ShareContactCardRequested,
        UiEventKind::CreateGroupRequested {
            context_id: ContextId::new("g-new"),
            title: "Team".to_owned(),
            members: vec![ParticipantId::new("p-1")],
        },
        UiEventKind::RenameRequested { title: "Crew".to_owned() },
        UiEventKind::ContactActionPressed {
            participant_id: ParticipantId::new("p-1"),
            action: ContactAction::Block,
        },
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
        UiEventKind::AgentApprovalPressed {
            owner_id: AccountId::new("a-1"),
            approval_id: "ap-1".to_owned(),
            approved: true,
        },
        UiEventKind::AddNodeRequested { url: "https://node.example.test".into() },
        UiEventKind::ConfirmNodePressed { node_id: NodeId::new("n-1") },
        UiEventKind::NodeActionPressed {
            node_id: NodeId::new("n-1"),
            action: NodeAction::SetEnabled(false),
        },
        UiEventKind::LockPressed { account_id: AccountId::new("a-1") },
        UiEventKind::AppLockConfigureRequested {
            account_id: AccountId::new("a-1"),
            enabled: false,
            timeout_ms: 0,
        },
        UiEventKind::RevokeDevicePressed { device_id: DeviceId::new("dev-2") },
        UiEventKind::DeviceSyncRequested {
            direction: SyncDirection::Export,
            path: "/tmp/s".into(),
        },
        UiEventKind::DiagnosticsExportRequested { path: "/tmp/d".into() },
        UiEventKind::FieldChanged { value: "x".into() },
        UiEventKind::FocusMomentRequested { moment_id: "m-1".into() },
        UiEventKind::CallStartRequested,
        UiEventKind::ContactRequestAnswered {
            request_id: "rq-1".to_owned(),
            answer: RequestAnswer::Decline,
        },
        UiEventKind::ContactRequestPolicySwitched { auto_accept: true },
        UiEventKind::ClearRequestFloorsPressed,
        UiEventKind::CallAnswered { call_id: "k-1".into(), accept: false },
        UiEventKind::CallHangUpPressed { call_id: "k-1".into() },
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
            .confirmed_by_human(ConfirmationIssuer::new().issue_tap());
        let first = serde_json::to_value(&intent).expect("serialize intent");
        let back: Intent = serde_json::from_value(first.clone()).expect("deserialize intent");
        let mut second = serde_json::to_value(&back).expect("re-serialize intent");
        second["confirmed_by"] = first["confirmed_by"].clone();
        assert_eq!(first, second, "everything but the confirmation round-trips");
        assert_eq!(back.proposed_by, intent.proposed_by);
        assert!(intent.confirmed_by.is_some());
        assert!(back.confirmed_by.is_none(), "a deserialized intent is never confirmed");
        assert_eq!(first["confirmed_by"]["gesture"], "Tap");
    }
}

#[test]
fn secret_intents_refuse_to_serialize() {
    let secret = || SecretInput::new("correct horse");
    let kinds = [
        IntentKind::Unlock { account_id: AccountId::new("a-1"), secret: secret() },
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
        IntentKind::CreateBackup {
            account_id: AccountId::new("a-1"),
            path: "/tmp/b".into(),
            secret: secret(),
            allow_missing_attachments: false,
        },
        IntentKind::VerifyRestore { path: "/tmp/b".into(), secret: secret() },
        IntentKind::ClaimUsername {
            account_id: AccountId::new("a-1"),
            name: "ana".into(),
            secret: secret(),
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
        "CreateBackup",
        "VerifyRestore",
        "ClaimUsername",
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
