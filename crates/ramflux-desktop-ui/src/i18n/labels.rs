// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Human names of contract enums, one function per enum.

use gpui_kit::SharedString;
use ramflux_desktop_core::{
    AccountKind, Direction, GovernanceAction, GrantState, Lifecycle, MomentKind, NodeHealth,
    PeerKind, PinSource, Receipt, Risk, Role, SafetyState, SourceStatus, StepState, SystemLevel,
    TaskState, TransferState,
};

use super::tr;

/// Label of the source connection state.
pub fn source_status(status: SourceStatus) -> SharedString {
    tr(match status {
        SourceStatus::Connected => "status.source.connected",
        SourceStatus::Disconnected => "status.source.disconnected",
        SourceStatus::Locked => "status.source.locked",
    })
}

/// Label of a node health state.
pub fn node_health(health: NodeHealth) -> SharedString {
    tr(match health {
        NodeHealth::Unknown => "status.node.unknown",
        NodeHealth::Healthy => "status.node.healthy",
        NodeHealth::Degraded => "status.node.degraded",
        NodeHealth::Unreachable => "status.node.unreachable",
    })
}

/// Label of a node pin source.
pub fn pin_source(source: PinSource) -> SharedString {
    tr(match source {
        PinSource::Bootstrap => "node.pin.bootstrap",
        PinSource::Invite => "node.pin.invite",
        PinSource::Manual => "node.pin.manual",
    })
}

/// Label of a moment lifecycle.
pub fn lifecycle(lifecycle: Lifecycle) -> SharedString {
    tr(match lifecycle {
        Lifecycle::Pending => "lifecycle.pending",
        Lifecycle::InProgress => "lifecycle.in_progress",
        Lifecycle::NeedsMe => "lifecycle.needs_me",
        Lifecycle::Done => "lifecycle.done",
        Lifecycle::Failed => "lifecycle.failed",
        Lifecycle::Dismissed => "lifecycle.dismissed",
    })
}

/// Label of a task state.
pub fn task_state(state: TaskState) -> SharedString {
    tr(match state {
        TaskState::Proposed => "task_state.proposed",
        TaskState::Running => "task_state.running",
        TaskState::NeedsMe => "task_state.needs_me",
        TaskState::Done => "task_state.done",
        TaskState::Failed => "task_state.failed",
        TaskState::Cancelled => "task_state.cancelled",
    })
}

/// Label of a task step state.
pub fn step_state(state: StepState) -> SharedString {
    tr(match state {
        StepState::Pending => "step_state.pending",
        StepState::Running => "step_state.running",
        StepState::NeedsMe => "step_state.needs_me",
        StepState::Done => "step_state.done",
        StepState::Failed => "step_state.failed",
        StepState::Skipped => "step_state.skipped",
    })
}

/// Label of a risk level.
pub fn risk(risk: Risk) -> SharedString {
    tr(match risk {
        Risk::Low => "risk.low",
        Risk::Medium => "risk.medium",
        Risk::High => "risk.high",
        Risk::Critical => "risk.critical",
    })
}

/// Label of a system moment level.
pub fn system_level(level: SystemLevel) -> SharedString {
    tr(match level {
        SystemLevel::Info => "moment.system.info",
        SystemLevel::Warning => "moment.system.warning",
        SystemLevel::Error => "moment.system.error",
    })
}

/// Label of a delivery receipt.
pub fn receipt(receipt: Receipt) -> SharedString {
    tr(match receipt {
        Receipt::Sending => "moment.message.sending",
        Receipt::Sent => "moment.message.sent",
        Receipt::Delivered => "moment.message.delivered",
        Receipt::Read => "moment.message.read",
    })
}

/// Label of a member role.
pub fn role(role: Role) -> SharedString {
    tr(match role {
        Role::Owner => "role.owner",
        Role::Admin => "role.admin",
        Role::Member => "role.member",
        Role::Bot => "role.bot",
    })
}

/// Label of a grant state.
pub fn grant_state(state: GrantState) -> SharedString {
    tr(match state {
        GrantState::Pending => "grant.state.pending",
        GrantState::Approved => "grant.state.approved",
        GrantState::Denied => "grant.state.denied",
        GrantState::Expired => "grant.state.expired",
        GrantState::Revoked => "grant.state.revoked",
    })
}

/// Label of a participant safety state.
pub fn safety(state: SafetyState) -> SharedString {
    tr(match state {
        SafetyState::Unverified => "safety.unverified",
        SafetyState::Verified => "safety.verified",
        SafetyState::KeyChanged => "safety.key_changed",
        SafetyState::Forked => "safety.forked",
        SafetyState::VerificationStale => "safety.verification_stale",
    })
}

/// Label of a peer kind (what a profile or card states a principal is).
pub fn peer_kind(kind: PeerKind) -> SharedString {
    tr(match kind {
        PeerKind::Human => "identity.kind.human",
        PeerKind::HeadlessAi => "identity.kind.headless_ai",
        PeerKind::Bot => "identity.kind.bot",
    })
}

/// Label of an account kind.
pub fn account_kind(kind: AccountKind) -> SharedString {
    tr(match kind {
        AccountKind::HumanInteractive => "account.kind.human",
        AccountKind::HeadlessAi => "account.kind.headless",
    })
}

/// Label of an object direction.
pub fn direction(direction: Direction) -> SharedString {
    tr(match direction {
        Direction::Inbound => "object.inbound",
        Direction::Outbound => "object.outbound",
    })
}

/// Label of a transfer state.
pub fn transfer(state: &TransferState) -> SharedString {
    tr(match state {
        TransferState::Queued => "object.queued",
        TransferState::Transferring { .. } => "object.transferring",
        TransferState::Complete => "object.complete",
        TransferState::Failed { .. } => "object.failed",
    })
}

/// Label of a governance action.
pub fn governance_action(action: GovernanceAction) -> SharedString {
    tr(match action {
        GovernanceAction::Invited => "moment.governance.invited",
        GovernanceAction::Joined => "moment.governance.joined",
        GovernanceAction::Left => "moment.governance.left",
        GovernanceAction::Removed => "moment.governance.removed",
        GovernanceAction::Banned => "moment.governance.banned",
        GovernanceAction::RoleChanged => "moment.governance.role_changed",
    })
}

/// Human name of a moment kind.
pub fn moment_kind(kind: &MomentKind) -> SharedString {
    tr(match kind {
        MomentKind::Message(_) => "moment.kind.message",
        MomentKind::AgentSurface(_) => "moment.kind.agent_surface",
        MomentKind::ObjectTransfer(_) => "moment.kind.object_transfer",
        MomentKind::Intervention(_) => "moment.kind.intervention",
        MomentKind::SafetyChange(_) => "moment.kind.safety_change",
        MomentKind::Governance(_) => "moment.kind.governance",
        MomentKind::ContactRequest(_) => "moment.kind.contact_request",
        MomentKind::TaskProgress(_) => "moment.kind.task_progress",
        MomentKind::TaskEvent(_) => "moment.kind.task_event",
        MomentKind::Conflict(_) => "moment.kind.conflict",
        MomentKind::PrivacyReveal(_) => "moment.kind.privacy_reveal",
        MomentKind::Handoff(_) => "moment.kind.handoff",
        MomentKind::Fallback(_) => "moment.kind.fallback",
        MomentKind::System(_) => "moment.kind.system",
        MomentKind::Call(_) => "moment.kind.call",
        MomentKind::NameChange(_) => "moment.kind.name_change",
        MomentKind::GroupRename(_) => "moment.kind.group_rename",
    })
}
