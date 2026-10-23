// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Technical names of contract variants for the event inspector.
//!
//! These are trace labels (variant and method names), never user-facing text
//! and never secrets.

use ramflux_desktop_core::{DomainEvent, Effect, Intent, IntentKind, Scope, UiEvent, UiEventKind};

/// Formats a scope path as `Workspace > Context(c-1) > Moment(m-1)`.
pub fn format_path(path: &[Scope]) -> String {
    path.iter().map(format_scope).collect::<Vec<_>>().join(" > ")
}

/// Formats one scope.
pub fn format_scope(scope: &Scope) -> String {
    match scope {
        Scope::Workspace => "Workspace".to_owned(),
        Scope::Panel(name) => format!("Panel({name})"),
        Scope::Context(id) => format!("Context({id})"),
        Scope::Moment(id) => format!("Moment({id})"),
        Scope::Surface(id) => format!("Surface({id})"),
        Scope::Block(id) => format!("Block({id})"),
    }
}

/// Variant name of a UI event.
pub fn ui_event_name(event: &UiEvent) -> &'static str {
    match event.kind {
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
        UiEventKind::MuteToggled { .. } => "MuteToggled",
        UiEventKind::DisappearingChosen { .. } => "DisappearingChosen",
        UiEventKind::ContactAnswered { .. } => "ContactAnswered",
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
        UiEventKind::ExportBackupRequested { .. } => "ExportBackupRequested",
        UiEventKind::RevokeDevicePressed { .. } => "RevokeDevicePressed",
        UiEventKind::DeviceSyncRequested { .. } => "DeviceSyncRequested",
        UiEventKind::DiagnosticsExportRequested { .. } => "DiagnosticsExportRequested",
        UiEventKind::ExpandToggled { .. } => "ExpandToggled",
        UiEventKind::FieldChanged { .. } => "FieldChanged",
    }
}

/// Variant name of an intent.
pub fn intent_name(intent: &Intent) -> &'static str {
    intent_kind_name(&intent.kind)
}

/// Variant name of an intent kind.
pub fn intent_kind_name(kind: &IntentKind) -> &'static str {
    match kind {
        IntentKind::SendMessage { .. } => "SendMessage",
        IntentKind::AttachObject { .. } => "AttachObject",
        IntentKind::PinContext { .. } => "PinContext",
        IntentKind::UnpinContext { .. } => "UnpinContext",
        IntentKind::MuteContext { .. } => "MuteContext",
        IntentKind::SetDisappearing { .. } => "SetDisappearing",
        IntentKind::StartTask { .. } => "StartTask",
        IntentKind::CancelTask { .. } => "CancelTask",
        IntentKind::SurfaceAction { .. } => "SurfaceAction",
        IntentKind::Approve { .. } => "Approve",
        IntentKind::Deny { .. } => "Deny",
        IntentKind::Reveal { .. } => "Reveal",
        IntentKind::ResolveConflict { .. } => "ResolveConflict",
        IntentKind::AcceptContact { .. } => "AcceptContact",
        IntentKind::RejectContact { .. } => "RejectContact",
        IntentKind::VerifySafetyNumber { .. } => "VerifySafetyNumber",
        IntentKind::AcceptHandoff { .. } => "AcceptHandoff",
        IntentKind::DeclineHandoff { .. } => "DeclineHandoff",
        IntentKind::PullObject { .. } => "PullObject",
        IntentKind::SaveObject { .. } => "SaveObject",
        IntentKind::RetryTransfer { .. } => "RetryTransfer",
        IntentKind::Governance { .. } => "Governance",
        IntentKind::RevokeGrant { .. } => "RevokeGrant",
        IntentKind::RevokeStandingApproval { .. } => "RevokeStandingApproval",
        IntentKind::Unlock { .. } => "Unlock",
        IntentKind::CreateAccount { .. } => "CreateAccount",
        IntentKind::ActivateDevice { .. } => "ActivateDevice",
        IntentKind::RestoreBackup { .. } => "RestoreBackup",
        IntentKind::AddNode { .. } => "AddNode",
        IntentKind::ConfirmNode { .. } => "ConfirmNode",
        IntentKind::LockAccount { .. } => "LockAccount",
        IntentKind::RotatePassphrase { .. } => "RotatePassphrase",
        IntentKind::ExportBackup { .. } => "ExportBackup",
        IntentKind::RevokeDevice { .. } => "RevokeDevice",
        IntentKind::SyncDevices { .. } => "SyncDevices",
        IntentKind::ProbeNode { .. } => "ProbeNode",
        IntentKind::RemoveNode { .. } => "RemoveNode",
        IntentKind::SetDefaultNode { .. } => "SetDefaultNode",
        IntentKind::SetNodeEnabled { .. } => "SetNodeEnabled",
        IntentKind::ExportDiagnostics { .. } => "ExportDiagnostics",
    }
}

/// Label of an effect; never includes secret material.
pub fn effect_label(effect: &Effect) -> String {
    match effect {
        Effect::Call { method, .. } => method.clone(),
        Effect::SecretCall { method, secrets, .. } => {
            format!("{method} (+{} secret)", secrets.len())
        }
        Effect::RequireConfirmation { intent } => {
            format!("RequireConfirmation({})", intent_name(intent))
        }
    }
}

/// Variant name of a domain event.
pub fn event_name(event: &DomainEvent) -> &'static str {
    match event {
        DomainEvent::AccountUpdated(_) => "AccountUpdated",
        DomainEvent::AccountLockChanged { .. } => "AccountLockChanged",
        DomainEvent::NodeUpserted(_) => "NodeUpserted",
        DomainEvent::NodeHealthChanged { .. } => "NodeHealthChanged",
        DomainEvent::NodeSettingsChanged { .. } => "NodeSettingsChanged",
        DomainEvent::NodeProbed { .. } => "NodeProbed",
        DomainEvent::NodeRemoved { .. } => "NodeRemoved",
        DomainEvent::DeviceUpserted(_) => "DeviceUpserted",
        DomainEvent::ParticipantUpserted(_) => "ParticipantUpserted",
        DomainEvent::ContextUpserted(_) => "ContextUpserted",
        DomainEvent::ContextSettingsChanged { .. } => "ContextSettingsChanged",
        DomainEvent::ContextRemoved { .. } => "ContextRemoved",
        DomainEvent::MomentAdded(_) => "MomentAdded",
        DomainEvent::MomentUpdated { .. } => "MomentUpdated",
        DomainEvent::SurfacePatched { .. } => "SurfacePatched",
        DomainEvent::ObjectUpserted(_) => "ObjectUpserted",
        DomainEvent::GrantRequested(_) => "GrantRequested",
        DomainEvent::GrantResolved { .. } => "GrantResolved",
        DomainEvent::GrantRevoked { .. } => "GrantRevoked",
        DomainEvent::StandingApprovalUpserted(_) => "StandingApprovalUpserted",
        DomainEvent::AuditAppended(_) => "AuditAppended",
        DomainEvent::IntegrationsUpdated(_) => "IntegrationsUpdated",
        DomainEvent::TaskUpserted(_) => "TaskUpserted",
        DomainEvent::TaskStepChanged { .. } => "TaskStepChanged",
        DomainEvent::SourceStatusChanged(_) => "SourceStatusChanged",
    }
}

#[cfg(test)]
mod tests {
    use ramflux_desktop_core::{ContextId, SecretInput, methods};
    use serde_json::json;

    use super::*;

    #[test]
    fn secret_calls_never_show_the_secret() {
        let effect = Effect::SecretCall {
            method: methods::ACCOUNT_UNLOCK.to_owned(),
            body: json!({ "account_id": "a-1" }),
            secrets: vec![SecretInput::new("river stone")],
        };
        assert_eq!(effect_label(&effect), "account.unlock (+1 secret)");
        assert!(!format!("{effect:?}").contains("river"));
    }

    #[test]
    fn paths_render_root_first() {
        let path = [
            Scope::Workspace,
            Scope::Panel(format!("dock.{}", "members")),
            Scope::Context(ContextId::new("c-1")),
        ];
        assert_eq!(format_path(&path), "Workspace > Panel(dock.members) > Context(c-1)");
    }
}
