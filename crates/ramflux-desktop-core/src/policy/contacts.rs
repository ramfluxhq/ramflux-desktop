// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for contacts: adding one by principal id or contact card,
//! exporting this account's own card, and acting on a contact link (accept,
//! reject, block, unblock, remove).

use serde_json::json;

use super::{Decision, IntentError, Policy};
use crate::intent::{ContactAction, IntentKind, check_contact_card, check_contact_id, methods};
use crate::model::{
    ContactLink, RequestAnswer, RequestDirection, RequestPolicy, RequestState, Risk,
};

/// Decides contact-area kinds; `None` for kinds of other areas.
pub(super) fn decide(p: &Policy<'_>, kind: &IntentKind) -> Option<Result<Decision, IntentError>> {
    Some(match kind {
        IntentKind::AddContact { participant_id } => p.human_only("add a contact").and_then(|()| {
            check_add_contact(p.store, participant_id.as_str())?;
            Ok(Decision::Call(methods::CONTACT_ADD, json!({ "participant_id": participant_id })))
        }),
        IntentKind::AddContactCard { card, replace_provisional_root } => {
            p.human_only("add a contact").and_then(|()| {
                let card = check_add_contact_card(card)?;
                let mut body = json!({ "card": card });
                if let Some(root) = replace_provisional_root.as_deref().filter(|r| !r.is_empty()) {
                    body["replace_provisional_root"] = json!(root);
                }
                Ok(Decision::Call(methods::CONTACT_CARD_IMPORT, body))
            })
        }
        IntentKind::ExportContactCard => p
            .human_only("share the contact card")
            .map(|()| Decision::Call(methods::CONTACT_CARD_EXPORT, json!({}))),
        IntentKind::AnswerContactRequest { request_id, answer } => {
            p.human_only("answer a contact request").and_then(|()| {
                let request = p
                    .store
                    .contact_request_by_id(request_id)
                    .filter(|r| r.direction == RequestDirection::Incoming)
                    .ok_or_else(|| IntentError::UnknownContactRequest(request_id.clone()))?;
                if !request.awaits_answer() {
                    return Err(IntentError::ContactRequestNotPending(request_id.clone()));
                }
                // The daemon is sent the request id only.
                let body = json!({ "request_id": request_id });
                Ok(match answer {
                    RequestAnswer::Accept => Decision::Call(methods::CONTACT_REQUEST_ACCEPT, body),
                    RequestAnswer::Decline => {
                        Decision::Call(methods::CONTACT_REQUEST_DECLINE, body)
                    }
                    // Blocking drops whatever the requester sends from now
                    // on: it needs the hold, as blocking a contact does.
                    RequestAnswer::Block => {
                        p.gated(Risk::High, methods::CONTACT_REQUEST_BLOCK, body)
                    }
                })
            })
        }
        IntentKind::SetContactRequestPolicy { policy } => {
            p.human_only("change the contact request setting").map(|()| {
                let value = match policy {
                    RequestPolicy::Manual => "manual",
                    RequestPolicy::AutoAccept => "auto_accept",
                };
                Decision::Call(
                    methods::CONTACT_REQUESTS_POLICY,
                    json!({ "contact_requests": value }),
                )
            })
        }
        IntentKind::ClearRequestFloors => {
            p.human_only("forget old contact request entries").and_then(|()| {
                // The daemon takes the account's current setting with the
                // clear action, so that the setting stays as it is.
                let value = match p.store.request_policy() {
                    Some(RequestPolicy::Manual) => "manual",
                    Some(RequestPolicy::AutoAccept) => "auto_accept",
                    None => return Err(IntentError::ContactRequestSettingUnknown),
                };
                if p.store.request_floors().is_none_or(|floors| floors.clearable == 0) {
                    return Err(IntentError::NothingToForget);
                }
                // It forgets the rollback protection of people whose request
                // is closed: it needs the hold.
                Ok(p.gated(
                    Risk::High,
                    methods::CONTACT_REQUESTS_POLICY,
                    json!({ "contact_requests": value, "clear_retired_authority_floors": true }),
                ))
            })
        }
        IntentKind::Contact { participant_id, action } => p
            .store
            .participant(participant_id)
            .ok_or_else(|| IntentError::UnknownParticipant(participant_id.clone()))
            .and_then(|participant| {
                let link = participant.link_state();
                if !action.applies_to(link) {
                    return Err(IntentError::ContactLinkState {
                        participant: participant_id.clone(),
                        link,
                    });
                }
                Ok(p.gated(
                    action.risk(),
                    action.method(),
                    json!({ "participant_id": participant_id }),
                ))
            }),
        _ => return None,
    })
}

/// Checks a contact id the user typed before it is added: a usable id, and
/// no accepted, blocked or incoming link with it (those are answered with
/// their own actions; adding again would silently override them), except
/// asking again when they have not accepted this account's request. The form
/// runs the same check to explain a refusal next to the field.
///
/// # Errors
///
/// [`IntentError::InvalidContactId`] or [`IntentError::ContactLinkState`].
pub fn check_add_contact(
    store: &crate::store::Store,
    id: &str,
) -> Result<crate::ids::ParticipantId, IntentError> {
    check_contact_id(id).map_err(IntentError::InvalidContactId)?;
    let participant = crate::ids::ParticipantId::new(id);
    let mut link = store.participant(&participant).map_or(ContactLink::None, |c| c.link_state());
    // A pending request from them is answered, not added over.
    if store
        .contact_request(&participant, RequestDirection::Incoming)
        .is_some_and(crate::model::ContactRequest::awaits_answer)
    {
        link = ContactLink::Incoming;
    }
    // This account's own request they have not accepted (pending, expired
    // or declined) is asked again explicitly: the daemon resends the same
    // request while it is pending, and refuses a new one while a decline
    // cools down. It never retries on its own.
    let asked_again = link == ContactLink::Accepted
        && store
            .contact_request(&participant, RequestDirection::Outgoing)
            .is_some_and(|r| r.state != RequestState::Accepted);
    if !asked_again
        && matches!(link, ContactLink::Accepted | ContactLink::Blocked | ContactLink::Incoming)
    {
        return Err(IntentError::ContactLinkState { participant, link });
    }
    Ok(participant)
}

/// Checks contact card text before it is added; returns it trimmed. The
/// form runs the same check to explain a refusal next to the field.
///
/// # Errors
///
/// [`IntentError::InvalidContactCard`].
pub fn check_add_contact_card(card: &str) -> Result<&str, IntentError> {
    let card = card.trim();
    check_contact_card(card).map_err(IntentError::InvalidContactCard)?;
    Ok(card)
}

/// Whether a pending contact intent still matches the store (the link did
/// not change since the user asked).
pub fn contact_action_current(
    store: &crate::store::Store,
    participant: &crate::ids::ParticipantId,
    action: ContactAction,
) -> bool {
    store.participant(participant).is_some_and(|p| action.applies_to(p.link_state()))
}
