// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for decisions taken inside moments: surface actions, approvals,
//! reveals, conflicts, contact requests, safety and handoffs.

use serde_json::json;

use super::{Decision, IntentError, Policy};
use crate::conformance::SurfaceHash;
use crate::ids::{GrantId, MomentId};
use crate::intent::{IntentKind, methods};
use crate::model::{Grant, GrantState, Lifecycle, MomentKind, Risk, SafetyState};
use crate::store::SurfaceState;

/// Decides moment-area kinds; `None` for kinds of other areas.
pub(super) fn decide(p: &Policy<'_>, kind: &IntentKind) -> Option<Result<Decision, IntentError>> {
    Some(match kind {
        IntentKind::SurfaceAction {
            moment_id,
            surface_id,
            action_id,
            values,
            surface_hash,
            version,
        } => {
            let shown = Shown { hash: surface_hash, version: *version };
            surface_action(p, moment_id, surface_id, action_id, &shown).map(|risk| {
                p.gated(
                    risk,
                    methods::SURFACE_ACTION,
                    json!({
                        "moment_id": moment_id,
                        "surface_id": surface_id,
                        "version": version,
                        "surface_hash": surface_hash,
                        "action_id": action_id,
                        "values": values,
                    }),
                )
            })
        }
        IntentKind::RequestSurfaceUpdate { moment_id, surface_id } => {
            p.human_only("ask for a surface's current state").and_then(|()| {
                let record = p
                    .store
                    .surface(moment_id)
                    .filter(|r| &r.surface_id == surface_id)
                    .ok_or_else(|| wrong_kind(moment_id, "AgentSurface"))?;
                if record.state != SurfaceState::OutOfSync {
                    return Err(IntentError::SurfaceInSync(moment_id.clone()));
                }
                Ok(Decision::Call(
                    methods::SURFACE_REQUEST_RESYNC,
                    json!({ "moment_id": moment_id, "surface_id": surface_id }),
                ))
            })
        }
        IntentKind::Approve { grant_id } => open_grant(p, grant_id).map(|grant| {
            p.gated(
                grant.risk,
                methods::GRANT_APPROVE,
                json!({ "grant_id": grant_id, "args_hash": grant.args_hash }),
            )
        }),
        IntentKind::Deny { grant_id } => open_grant(p, grant_id)
            .map(|_| Decision::Call(methods::GRANT_DENY, json!({ "grant_id": grant_id }))),
        IntentKind::Reveal { moment_id } => p.moment(moment_id).and_then(|moment| {
            let MomentKind::PrivacyReveal(payload) = &moment.kind else {
                return Err(wrong_kind(moment_id, "PrivacyReveal"));
            };
            if payload.revealed.is_some() || moment.lifecycle.is_terminal() {
                return Err(IntentError::NotActionable(moment_id.clone()));
            }
            // A reveal exposes protected content: always a hold, always audited.
            Ok(p.gated(Risk::High, methods::PRIVACY_REVEAL, json!({ "moment_id": moment_id })))
        }),
        IntentKind::ResolveConflict { moment_id, resolution } => {
            p.moment(moment_id).and_then(|moment| {
                let MomentKind::Conflict(payload) = &moment.kind else {
                    return Err(wrong_kind(moment_id, "Conflict"));
                };
                if payload.resolution.is_some() || moment.lifecycle.is_terminal() {
                    return Err(IntentError::NotActionable(moment_id.clone()));
                }
                Ok(Decision::Call(
                    methods::CONFLICT_RESOLVE,
                    json!({
                        "moment_id": moment_id,
                        "object_id": payload.object_id,
                        "resolution": resolution,
                    }),
                ))
            })
        }
        IntentKind::AcceptContact { moment_id } | IntentKind::RejectContact { moment_id } => {
            let accept = matches!(kind, IntentKind::AcceptContact { .. });
            p.moment(moment_id).and_then(|moment| {
                let MomentKind::ContactRequest(payload) = &moment.kind else {
                    return Err(wrong_kind(moment_id, "ContactRequest"));
                };
                needs_me(moment_id, moment.lifecycle)?;
                let method = if accept { methods::CONTACT_ACCEPT } else { methods::CONTACT_REJECT };
                Ok(Decision::Call(method, json!({ "moment_id": moment_id, "from": payload.from })))
            })
        }
        IntentKind::VerifySafetyNumber { participant_id } => {
            p.human_only("verify a safety number").and_then(|()| {
                let participant = p
                    .store
                    .participant(participant_id)
                    .ok_or_else(|| IntentError::UnknownParticipant(participant_id.clone()))?;
                if participant.safety == SafetyState::Verified {
                    return Err(IntentError::AlreadyVerified(participant_id.clone()));
                }
                Ok(Decision::Call(
                    methods::SAFETY_VERIFY,
                    json!({ "participant_id": participant_id }),
                ))
            })
        }
        IntentKind::AcceptHandoff { moment_id } | IntentKind::DeclineHandoff { moment_id } => {
            let accept = matches!(kind, IntentKind::AcceptHandoff { .. });
            p.human_only("answer a handoff").and_then(|()| {
                let moment = p.moment(moment_id)?;
                let MomentKind::Handoff(payload) = &moment.kind else {
                    return Err(wrong_kind(moment_id, "Handoff"));
                };
                needs_me(moment_id, moment.lifecycle)?;
                let method =
                    if accept { methods::HANDOFF_ACCEPT } else { methods::HANDOFF_DECLINE };
                Ok(Decision::Call(
                    method,
                    json!({ "moment_id": moment_id, "from_device": payload.from_device }),
                ))
            })
        }
        _ => return None,
    })
}

fn wrong_kind(moment: &MomentId, expected: &'static str) -> IntentError {
    IntentError::WrongMomentKind { moment: moment.clone(), expected }
}

fn needs_me(moment_id: &MomentId, lifecycle: Lifecycle) -> Result<(), IntentError> {
    if lifecycle == Lifecycle::NeedsMe {
        Ok(())
    } else {
        Err(IntentError::NotActionable(moment_id.clone()))
    }
}

/// The surface version an action was shown on.
struct Shown<'a> {
    hash: &'a SurfaceHash,
    version: u64,
}

/// Validates a surface action and returns its declared risk.
///
/// The action must have been shown on the current surface version (`shown`:
/// its content hash, and its version number when the view knew it), on a
/// surface that is open: not closed, not in conflict, within the limits.
fn surface_action(
    p: &Policy<'_>,
    moment_id: &MomentId,
    surface_id: &crate::ids::SurfaceId,
    action_id: &str,
    shown: &Shown<'_>,
) -> Result<Risk, IntentError> {
    let moment = p.moment(moment_id)?;
    let MomentKind::AgentSurface(surface) = &moment.kind else {
        return Err(wrong_kind(moment_id, "AgentSurface"));
    };
    let record = p.store.surface(moment_id).ok_or_else(|| wrong_kind(moment_id, "AgentSurface"))?;
    match record.state {
        SurfaceState::Closed => return Err(IntentError::SurfaceClosed(moment_id.clone())),
        SurfaceState::Conflict { .. } => {
            return Err(IntentError::SurfaceConflict(moment_id.clone()));
        }
        SurfaceState::OverLimit(_) => {
            return Err(IntentError::SurfaceOverLimit(moment_id.clone()));
        }
        SurfaceState::OutOfSync => return Err(IntentError::SurfaceOutOfSync(moment_id.clone())),
        SurfaceState::Open => {}
    }
    let version_known = shown.version != 0;
    if &record.hash != shown.hash || (version_known && record.version != shown.version) {
        return Err(IntentError::SurfaceChanged(moment_id.clone()));
    }
    let action = surface
        .actions
        .iter()
        .find(|a| &surface.surface_id == surface_id && a.id == action_id)
        .ok_or_else(|| IntentError::UnknownAction {
            moment: moment_id.clone(),
            action: action_id.to_owned(),
        })?;
    if let Some(reason) = &action.disabled_reason {
        return Err(IntentError::ActionDisabled {
            action: action_id.to_owned(),
            reason: reason.clone(),
        });
    }
    Ok(action.risk)
}

fn open_grant<'a>(p: &Policy<'a>, id: &GrantId) -> Result<&'a Grant, IntentError> {
    let grant = p.store.grant(id).ok_or_else(|| IntentError::UnknownGrant(id.clone()))?;
    if let Some(failure) = grant.failure {
        return Err(IntentError::GrantFailed { grant: id.clone(), failure });
    }
    if grant.state != GrantState::Pending {
        return Err(IntentError::GrantNotPending(id.clone()));
    }
    if grant.expiry.is_some_and(|expiry| expiry <= p.now) {
        return Err(IntentError::GrantExpired(id.clone()));
    }
    Ok(grant)
}
