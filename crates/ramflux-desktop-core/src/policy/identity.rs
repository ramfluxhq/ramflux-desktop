// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for a contact's identity: reading its state and a direct
//! conversation's security, and accepting a changed identity.
//!
//! Accepting is the user's decision alone: the attended human, naming the
//! anchor of the change the sheet showed. When the stored change names a
//! different anchor (or none), the intent is refused and the sheet is
//! rendered again, so a newer change is never accepted by a click that
//! showed an older one. Nothing accepts by itself.

use serde_json::json;

use super::{Decision, IntentError, Policy};
use crate::intent::{IntentKind, methods};
use crate::model::ContextKind;

/// Decides identity-area kinds; `None` for kinds of other areas.
pub(super) fn decide(p: &Policy<'_>, kind: &IntentKind) -> Option<Result<Decision, IntentError>> {
    Some(match kind {
        IntentKind::AcceptIdentityChange { participant_id, new_anchor } => {
            p.human_only("accept an identity change").and_then(|()| {
                let participant = p
                    .store
                    .participant(participant_id)
                    .ok_or_else(|| IntentError::UnknownParticipant(participant_id.clone()))?;
                let Some(change) = participant.pending_change.as_ref() else {
                    return Err(IntentError::NoPendingIdentityChange(participant_id.clone()));
                };
                if !participant.identity.is_changed() {
                    return Err(IntentError::NoPendingIdentityChange(participant_id.clone()));
                }
                if &change.new_anchor != new_anchor {
                    return Err(IntentError::IdentityChangedAgain(participant_id.clone()));
                }
                Ok(Decision::Call(
                    methods::IDENTITY_ACCEPT_CHANGE,
                    json!({ "participant_id": participant_id, "new_anchor": new_anchor }),
                ))
            })
        }
        IntentKind::ReadIdentityState { participant_id } => {
            p.human_only("read an identity state").and_then(|()| {
                p.store
                    .participant(participant_id)
                    .ok_or_else(|| IntentError::UnknownParticipant(participant_id.clone()))?;
                Ok(Decision::Call(
                    methods::IDENTITY_STATUS,
                    json!({ "participant_id": participant_id }),
                ))
            })
        }
        IntentKind::ReadConversationSecurity { context_id } => {
            p.human_only("read a conversation's security").and_then(|()| {
                if p.context(context_id)?.kind != ContextKind::Direct {
                    return Err(IntentError::CallNeedsDirect(context_id.clone()));
                }
                Ok(Decision::Call(
                    methods::CONVERSATION_SECURITY,
                    json!({ "context_id": context_id }),
                ))
            })
        }
        _ => return None,
    })
}
