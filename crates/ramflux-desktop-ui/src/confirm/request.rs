// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! What the confirm strip says about a pending intent: the object
//! and verb in plain words, the effective risk, and who proposed it.

use gpui_kit::SharedString;
use ramflux_desktop_core::{
    Actor, AgentRuntimeAction, ContactAction, DeleteScope, GovernanceOp, Intent, IntentKind,
    MomentKind, ParticipantId, RequestAnswer, Risk, Store,
};

use crate::i18n::{tr, tr_args};

/// What the strip shows about the pending request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmRequest {
    summary: SharedString,
    detail: Option<SharedString>,
    risk: Risk,
    proposed_by: Option<SharedString>,
    /// The `prx` file an install confirmation names (resolved once, for the
    /// sheet and for the intent that follows it).
    runtime_path: Option<String>,
    /// An install with no `prx` file found has nothing to confirm.
    confirmable: bool,
}

impl ConfirmRequest {
    /// Describes `intent` for the strip.
    pub fn describe(store: &Store, intent: &Intent) -> Self {
        let install = matches!(
            &intent.kind,
            IntentKind::AgentRuntime { action: AgentRuntimeAction::Install, .. }
        );
        let prx = install.then(ramflux_desktop_sdk::prx_host::resolved_prx);
        let runtime_path = prx
            .as_ref()
            .and_then(|found| found.as_ref().ok())
            .map(|path| path.display().to_string());
        let (summary, detail, risk) = describe_kind(store, &intent.kind, prx.as_ref());
        let proposed_by = match &intent.proposed_by {
            Actor::Human => None,
            Actor::Agent(id) | Actor::Member(id) => Some(display_name(store, id).into()),
        };
        let confirmable = !install || runtime_path.is_some();
        Self { summary, detail, risk, proposed_by, runtime_path, confirmable }
    }

    /// Whether Confirm may act: an install is confirmed for a `prx` file the
    /// sheet names, so it needs one.
    pub fn is_confirmable(&self) -> bool {
        self.confirmable
    }

    /// The `prx` file the sheet names, for an install.
    pub fn runtime_path(&self) -> Option<&str> {
        self.runtime_path.as_deref()
    }

    /// One-line description: object and verb.
    pub fn summary(&self) -> &SharedString {
        &self.summary
    }

    /// Evidence shown under the summary (a key fingerprint, a scope).
    pub fn detail(&self) -> Option<&SharedString> {
        self.detail.as_ref()
    }

    /// Risk level shown on the strip.
    pub fn risk(&self) -> Risk {
        self.risk
    }

    /// Whether the request is destructive (high or critical risk): its
    /// strip opens with the focus on Cancel, never on Confirm.
    pub fn is_destructive(&self) -> bool {
        self.risk.requires_confirmation()
    }

    /// Display name of the agent that proposed the intent, if an agent did.
    pub fn proposed_by(&self) -> Option<&SharedString> {
        self.proposed_by.as_ref()
    }
}

fn display_name(store: &Store, id: &ParticipantId) -> String {
    store.participant(id).map_or_else(|| id.short(), |p| p.display.clone())
}

fn describe_kind(
    store: &Store,
    kind: &IntentKind,
    prx: Option<&Result<std::path::PathBuf, ramflux_desktop_sdk::prx_host::ResolveError>>,
) -> (SharedString, Option<SharedString>, Risk) {
    match kind {
        IntentKind::Approve { grant_id } => {
            let grant = store.grant(grant_id);
            let summary = store
                .grant_presenters(grant_id)
                .find_map(|m| match &m.kind {
                    MomentKind::Intervention(p) => Some(p.summary.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| grant.map_or_else(|| grant_id.to_string(), |g| g.scope.clone()));
            (
                tr_args("confirm.approve", &[("summary", &summary)]),
                grant.map(|g| SharedString::from(g.scope.clone())),
                grant.map_or(Risk::High, |g| g.risk),
            )
        }
        IntentKind::SurfaceAction { moment_id, action_id, .. } => {
            let action = store.moment(moment_id).and_then(|m| match &m.kind {
                MomentKind::AgentSurface(s) => {
                    s.actions.iter().find(|a| &a.id == action_id).cloned()
                }
                _ => None,
            });
            let (label, risk) =
                action.map_or((action_id.clone(), Risk::High), |a| (a.label, a.risk));
            (tr_args("confirm.surface_action", &[("action", &label)]), None, risk)
        }
        IntentKind::Governance { context_id, op } => {
            let group = store
                .context(context_id)
                .map_or_else(|| context_id.to_string(), |c| c.title.clone());
            let target = op.target().map(|id| display_name(store, id)).unwrap_or_default();
            let key = match op {
                GovernanceOp::Kick { .. } => "confirm.governance.kick",
                GovernanceOp::SetRole { .. } => "confirm.governance.set_role",
                GovernanceOp::AddMember { .. } => "confirm.governance.add_member",
                GovernanceOp::RemoveMember { .. } => "confirm.governance.remove_member",
            };
            (tr_args(key, &[("member", &target), ("group", &group)]), None, op.risk())
        }
        IntentKind::DeleteMessage { moment_id, scope } => {
            let excerpt = store.body(moment_id).map(ramflux_desktop_navigation::row::list_text);
            let key = match scope {
                DeleteScope::Local => "confirm.message.delete_local",
                DeleteScope::Everyone => "confirm.message.delete_everyone",
            };
            (tr(key), excerpt, Risk::High)
        }
        IntentKind::LeaveGroup { context_id } => {
            let group = store
                .context(context_id)
                .map_or_else(|| context_id.to_string(), |c| c.title.clone());
            (tr_args("confirm.group.leave", &[("group", &group)]), None, Risk::High)
        }
        IntentKind::Reveal { moment_id } => {
            let label = store
                .moment(moment_id)
                .and_then(|m| match &m.kind {
                    MomentKind::PrivacyReveal(p) => Some(p.label.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            (
                tr_args("confirm.reveal", &[("label", &label)]),
                Some(tr("confirm.reveal_audit")),
                Risk::High,
            )
        }
        IntentKind::RevokeGrant { grant_id } => {
            let scope =
                store.grant(grant_id).map_or_else(|| grant_id.to_string(), |g| g.scope.clone());
            (tr_args("confirm.revoke_grant", &[("scope", &scope)]), None, Risk::High)
        }
        IntentKind::RevokeStandingApproval { approval_id } => {
            let scope = store
                .standing_approval(approval_id)
                .map_or_else(|| approval_id.to_string(), |a| a.scope.clone());
            (tr_args("confirm.revoke_standing", &[("scope", &scope)]), None, Risk::High)
        }
        IntentKind::RevokeDevice { device_id } => {
            // A device is named for a person, never by its id.
            let label = tr("settings.devices.other_device");
            (tr_args("confirm.revoke_device", &[("device", &label)]), None, Risk::High)
        }
        IntentKind::ConfirmNode { node_id, pin_source } => {
            let fingerprint = store.node(node_id).and_then(|n| n.fingerprint.clone());
            let risk = if pin_source.is_trust_on_first_use() { Risk::High } else { Risk::Medium };
            (
                tr_args("confirm.trust_node", &[("node", node_id.as_str())]),
                fingerprint.map(SharedString::from),
                risk,
            )
        }
        IntentKind::Contact { participant_id, action } => {
            // Only blocking and removing need a confirmation.
            let key = match action {
                ContactAction::Block => "confirm.contact.block",
                ContactAction::Remove => "confirm.contact.remove",
                _ => return (tr("confirm.generic"), None, action.risk()),
            };
            (
                tr_args(key, &[("contact", &display_name(store, participant_id))]),
                Some(SharedString::from(participant_id.to_string())),
                action.risk(),
            )
        }
        // Only Block needs a confirmation: it names who is blocked and what it does.
        IntentKind::AnswerContactRequest { request_id, answer: RequestAnswer::Block } => {
            let peer = store.contact_request_by_id(request_id).map(|r| r.peer.clone());
            let contact =
                peer.as_ref().map_or_else(|| request_id.clone(), |p| display_name(store, p));
            (
                tr_args("confirm.contact.request_block", &[("contact", &contact)]),
                Some(SharedString::from(
                    peer.map_or_else(|| request_id.clone(), |p| p.to_string()),
                )),
                Risk::High,
            )
        }
        // Forgetting the entries of closed requests gives up rollback
        // protection for those people: it needs the confirmation, and says so.
        IntentKind::ClearRequestFloors => {
            let count = store.request_floors().map_or(0, |f| f.clearable).to_string();
            (tr_args("confirm.contact.clear_floors", &[("count", &count)]), None, Risk::High)
        }
        // An install runs a program from this computer's search path: the
        // confirmation names the file that would run.
        IntentKind::AgentRuntime { action: AgentRuntimeAction::Install, .. } => {
            let path = match prx {
                Some(Ok(path)) => SharedString::from(path.display().to_string()),
                _ => tr("confirm.agent_runtime.no_prx"),
            };
            (tr("confirm.agent_runtime.install"), Some(path), Risk::High)
        }
        _ => (tr("confirm.generic"), None, Risk::High),
    }
}
