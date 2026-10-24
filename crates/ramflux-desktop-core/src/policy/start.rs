// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for starting a direct conversation with a contact.
//!
//! The first message creates the conversation. Only the attended human
//! starts one, only with an accepted contact (the daemon refuses a message
//! to anyone else), never with this account's own identity, and never while
//! the contact's key changed or forked without being verified again. The id
//! of the new conversation is fresh and must not name a known context.

use serde_json::json;

use super::{Decision, IntentError, Policy};
use crate::ids::ParticipantId;
use crate::intent::{IntentKind, methods};
use crate::model::{ContactLink, SafetyState};
use crate::store::Store;

/// Decides the start kind; `None` for other kinds.
pub(super) fn decide(p: &Policy<'_>, kind: &IntentKind) -> Option<Result<Decision, IntentError>> {
    let IntentKind::StartConversation { context_id, participant_id, text } = kind else {
        return None;
    };
    Some(p.human_only("start a conversation").and_then(|()| {
        if context_id.as_str().trim().is_empty() {
            return Err(IntentError::Empty("conversation id"));
        }
        if p.store.context(context_id).is_some() {
            return Err(IntentError::ContextExists(context_id.clone()));
        }
        if text.trim().is_empty() {
            return Err(IntentError::Empty("message text"));
        }
        check_start_conversation(p.store, participant_id)?;
        let display = p.store.participant(participant_id).map(|c| c.display.clone());
        Ok(Decision::Call(
            methods::CONVERSATION_START,
            json!({
                "context_id": context_id,
                "participant_id": participant_id,
                "text": text,
                "title": display,
            }),
        ))
    }))
}

/// Checks that a conversation can be started with `participant` now. The
/// new-conversation draft runs the same check to explain a refusal before
/// anything is typed.
///
/// # Errors
///
/// [`IntentError::UnknownParticipant`], [`IntentError::SelfParticipant`],
/// [`IntentError::ContactLinkState`] (not an accepted contact) or
/// [`IntentError::SafetyUnresolved`] (key changed or forked) or
/// [`IntentError::NotReachable`] (none of their devices is known yet).
pub fn check_start_conversation(
    store: &Store,
    participant: &ParticipantId,
) -> Result<(), IntentError> {
    let contact = store
        .participant(participant)
        .ok_or_else(|| IntentError::UnknownParticipant(participant.clone()))?;
    if crate::selectors::self_participant(store).as_ref() == Some(participant) {
        return Err(IntentError::SelfParticipant(participant.clone()));
    }
    let link = contact.link_state();
    if link != ContactLink::Accepted {
        return Err(IntentError::ContactLinkState { participant: participant.clone(), link });
    }
    if matches!(contact.safety, SafetyState::KeyChanged | SafetyState::Forked) {
        return Err(IntentError::SafetyUnresolved(participant.clone()));
    }
    if contact.reachable == Some(false) {
        return Err(IntentError::NotReachable(participant.clone()));
    }
    Ok(())
}
