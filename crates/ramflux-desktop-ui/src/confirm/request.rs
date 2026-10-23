// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! What the hold-to-confirm strip says about a pending intent: the object
//! and verb in plain words, the effective risk, and who proposed it.

use gpui_kit::SharedString;
use ramflux_desktop_core::{
    Actor, GovernanceOp, Intent, IntentKind, MomentKind, ParticipantId, Risk, Store,
};

use crate::i18n::{tr, tr_args};

/// What the strip shows about the pending request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmRequest {
    summary: SharedString,
    detail: Option<SharedString>,
    risk: Risk,
    proposed_by: Option<SharedString>,
}

impl ConfirmRequest {
    /// Describes `intent` for the strip.
    pub fn describe(store: &Store, intent: &Intent) -> Self {
        let (summary, detail, risk) = describe_kind(store, &intent.kind);
        let proposed_by = match &intent.proposed_by {
            Actor::Human => None,
            Actor::Agent(id) => Some(display_name(store, id).into()),
        };
        Self { summary, detail, risk, proposed_by }
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

    /// Display name of the agent that proposed the intent, if an agent did.
    pub fn proposed_by(&self) -> Option<&SharedString> {
        self.proposed_by.as_ref()
    }
}

fn display_name(store: &Store, id: &ParticipantId) -> String {
    store.participant(id).map_or_else(|| id.to_string(), |p| p.display.clone())
}

fn describe_kind(store: &Store, kind: &IntentKind) -> (SharedString, Option<SharedString>, Risk) {
    match kind {
        IntentKind::Approve { grant_id } => {
            let grant = store.grant(grant_id);
            let summary = store
                .moments()
                .find_map(|m| match &m.kind {
                    MomentKind::Intervention(p) if &p.grant_id == grant_id => {
                        Some(p.summary.clone())
                    }
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
                GovernanceOp::Ban { .. } => "confirm.governance.ban",
                GovernanceOp::SetRole { .. } => "confirm.governance.set_role",
                GovernanceOp::AddMember { .. } => "confirm.governance.add_member",
                GovernanceOp::RemoveMember { .. } => "confirm.governance.remove_member",
                GovernanceOp::CreateInvite { .. } => "confirm.governance.create_invite",
            };
            (tr_args(key, &[("member", &target), ("group", &group)]), None, op.risk())
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
            let label =
                store.device(device_id).map_or_else(|| device_id.to_string(), |d| d.label.clone());
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
        _ => (tr("confirm.generic"), None, Risk::High),
    }
}
