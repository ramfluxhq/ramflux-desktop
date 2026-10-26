// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for voice calls (CALL-01 §1, §6).
//!
//! Only the attended human places, answers, declines or hangs up a call;
//! an agent never does. A call is one-to-one, so it is placed only in a
//! direct conversation, to its other participant, never while that
//! participant's key changed or forked without being verified again, never
//! to a participant reported unreachable, and never while another call is
//! under way (one call at a time). Answering and declining apply to a call
//! that still rings here; hanging up to one that has not ended.

use serde_json::json;

use super::{Decision, IntentError, Policy};
use crate::ids::{CallId, ContextId, ParticipantId};
use crate::intent::{IntentKind, methods};
use crate::model::{Call, CallDirection, CallState, ContextKind};
use crate::selectors;
use crate::store::Store;

/// Decides the call kinds; `None` for other kinds.
pub(super) fn decide(p: &Policy<'_>, kind: &IntentKind) -> Option<Result<Decision, IntentError>> {
    let decision = match kind {
        IntentKind::StartCall { context_id } => {
            p.human_only("place a call").and_then(|()| start(p.store, context_id))
        }
        IntentKind::AnswerCall { call_id } => p.human_only("answer a call").and_then(|()| {
            let call = ringing(p.store, call_id)?;
            if selectors::current_call(p.store).is_some() {
                return Err(IntentError::CallInProgress);
            }
            Ok(Decision::Call(methods::CALL_ANSWER, json!({ "call_id": call.id })))
        }),
        IntentKind::RejectCall { call_id } => p.human_only("decline a call").and_then(|()| {
            let call = ringing(p.store, call_id)?;
            Ok(Decision::Call(methods::CALL_REJECT, json!({ "call_id": call.id })))
        }),
        IntentKind::HangUpCall { call_id } => p.human_only("hang up a call").and_then(|()| {
            let call = known(p.store, call_id)?;
            if call.state.is_ended() {
                return Err(IntentError::CallState(call_id.clone()));
            }
            Ok(Decision::Call(methods::CALL_HANGUP, json!({ "call_id": call.id })))
        }),
        _ => return None,
    };
    Some(decision)
}

fn start(store: &Store, context_id: &ContextId) -> Result<Decision, IntentError> {
    let peer = check_start_call(store, context_id)?;
    Ok(Decision::Call(methods::CALL_INVITE, json!({ "context_id": context_id, "peer": peer })))
}

fn known<'a>(store: &'a Store, call_id: &CallId) -> Result<&'a Call, IntentError> {
    store.call(call_id).ok_or_else(|| IntentError::UnknownCall(call_id.clone()))
}

fn ringing<'a>(store: &'a Store, call_id: &CallId) -> Result<&'a Call, IntentError> {
    let call = known(store, call_id)?;
    if call.direction != CallDirection::Incoming || call.state != CallState::IncomingRinging {
        return Err(IntentError::CallState(call_id.clone()));
    }
    Ok(call)
}

/// The other participant of a direct conversation, when there is exactly
/// one besides this account.
pub fn call_peer(store: &Store, context_id: &ContextId) -> Option<ParticipantId> {
    let context = store.context(context_id)?;
    let me = selectors::self_participant(store);
    let mut others = context.participants.iter().filter(|p| Some(*p) != me.as_ref());
    match (others.next(), others.next()) {
        (Some(peer), None) => Some(peer.clone()),
        _ => None,
    }
}

/// Checks that a call can be placed in `context_id` now and returns the
/// participant it would ring. The call button runs the same check to show
/// why it is disabled.
///
/// # Errors
///
/// [`IntentError::UnknownContext`], [`IntentError::CallNeedsDirect`] (not
/// a direct conversation, or no single other participant),
/// [`IntentError::SafetyUnresolved`], [`IntentError::NotReachable`] or
/// [`IntentError::CallInProgress`].
pub fn check_start_call(
    store: &Store,
    context_id: &ContextId,
) -> Result<ParticipantId, IntentError> {
    let context =
        store.context(context_id).ok_or_else(|| IntentError::UnknownContext(context_id.clone()))?;
    if context.kind != ContextKind::Direct {
        return Err(IntentError::CallNeedsDirect(context_id.clone()));
    }
    let peer = call_peer(store, context_id)
        .ok_or_else(|| IntentError::CallNeedsDirect(context_id.clone()))?;
    if let Some(participant) = store.participant(&peer) {
        if participant.safety.needs_attention() {
            return Err(IntentError::SafetyUnresolved(peer));
        }
        if participant.reachable == Some(false) {
            return Err(IntentError::NotReachable(peer));
        }
    }
    if selectors::call_under_way(store) {
        return Err(IntentError::CallInProgress);
    }
    Ok(peer)
}
