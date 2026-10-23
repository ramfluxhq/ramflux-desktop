// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for membership and roles, and for revoking grants and standing
//! approvals.

use serde_json::{Value, json};

use super::{Decision, IntentError, Policy};
use crate::ids::ContextId;
use crate::intent::{GovernanceOp, IntentKind, methods};
use crate::model::{ContextKind, GrantState, Risk};

/// Decides governance-area kinds; `None` for kinds of other areas.
pub(super) fn decide(p: &Policy<'_>, kind: &IntentKind) -> Option<Result<Decision, IntentError>> {
    Some(match kind {
        IntentKind::Governance { context_id, op } => check(p, context_id, op).map(|()| {
            let mut body = json!({ "context_id": context_id });
            if let Value::Object(map) = &mut body {
                if let Some(target) = op.target() {
                    map.insert("participant_id".to_owned(), json!(target));
                }
                if let Some(role) = op.role() {
                    map.insert("role".to_owned(), json!(role));
                }
            }
            p.gated(op.risk(), op.method(), body)
        }),
        IntentKind::RevokeGrant { grant_id } => p
            .store
            .grant(grant_id)
            .ok_or_else(|| IntentError::UnknownGrant(grant_id.clone()))
            .and_then(|grant| {
                if grant.state != GrantState::Approved {
                    return Err(IntentError::GrantNotApproved(grant_id.clone()));
                }
                Ok(p.gated(Risk::High, methods::GRANT_REVOKE, json!({ "grant_id": grant_id })))
            }),
        IntentKind::RevokeStandingApproval { approval_id } => p
            .store
            .standing_approval(approval_id)
            .ok_or_else(|| IntentError::UnknownStandingApproval(approval_id.clone()))
            .and_then(|approval| {
                if approval.revoked {
                    return Err(IntentError::StandingAlreadyRevoked(approval_id.clone()));
                }
                Ok(p.gated(
                    Risk::High,
                    methods::STANDING_REVOKE,
                    json!({ "approval_id": approval_id }),
                ))
            }),
        _ => return None,
    })
}

fn check(p: &Policy<'_>, context_id: &ContextId, op: &GovernanceOp) -> Result<(), IntentError> {
    let context = p.context(context_id)?;
    if context.kind == ContextKind::Direct {
        return Err(IntentError::NotAGroup(context_id.clone()));
    }
    let Some(target) = op.target() else {
        return Ok(());
    };
    if p.store.participant(target).is_none() {
        return Err(IntentError::UnknownParticipant(target.clone()));
    }
    let is_member = context.participants.contains(target);
    match op {
        GovernanceOp::AddMember { .. } if is_member => Err(IntentError::AlreadyMember {
            context: context_id.clone(),
            participant: target.clone(),
        }),
        GovernanceOp::RemoveMember { .. }
        | GovernanceOp::Kick { .. }
        | GovernanceOp::SetRole { .. }
            if !is_member =>
        {
            Err(IntentError::NotAMember {
                context: context_id.clone(),
                participant: target.clone(),
            })
        }
        _ => Ok(()),
    }
}
