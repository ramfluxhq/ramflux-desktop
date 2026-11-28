// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A contact's identity state and a direct conversation's security.
//!
//! | wire | desktop |
//! |---|---|
//! | `contact.verification.status` → `identity_state` `verified` / `tofu` / `changed` | `IdentityStateRead`; a token not known reads as `Tofu` (the token is logged, nothing else) |
//! | event `contact.identity_changed {contact, username, previous_anchor, new_anchor, ai_flag}` | `IdentityChanged`; the status is read again |
//! | `contact.trust.accept_change` → `{state: "accepted"}` | `IdentityAccepted` |
//! | refusal `no_pending_change` / `anchor_mismatch` / other | `IdentityAcceptFailed` `AlreadyHandled` / `ChangedAgain` / `Failed` |
//! | `conversation.security` → `{state, epoch, members[{anchor, device_id, ai_flag}], warning?}` | `ConversationSecurityRead`: `peer_devices` counts the members whose anchor is not this account's, `any_ai` is any `ai_flag` |
//! | event `conversation.devices_changed {conversation_id, added, removed}` | the security is read again; no moment |
//! | refusal `identity_change_pending {contact}` of a send | `SendBlockedByIdentity` |
//!
//! The `contact` identifier is the one the SDK used: it names a participant
//! by principal id or by identity commitment, and what the SDK named last is
//! sent back unchanged. Nothing here accepts a change or sends a message.

use ramflux_desktop_core::{
    AcceptFailure, ContextId, ConversationSecurity, DomainEvent, IdentityChange, IdentityState,
    ParticipantId, SecurityState, SecurityWarning, Timestamp,
};
use serde_json::Value;

use super::contacts::IdentityRecord;
use super::{Translator, text};
use crate::bus::BusError;
use crate::plan::fetch::Fetch;

/// The topic of a contact's identity change.
pub(crate) const IDENTITY_CHANGED: &str = "contact.identity_changed";

/// The topic of a conversation's device set change.
pub(crate) const DEVICES_CHANGED: &str = "conversation.devices_changed";

/// The refusal code of a send held back by an identity change.
const SEND_PENDING: &str = "identity_change_pending";

/// The state `identity_state` names; a token not known is read as `Tofu`.
pub(crate) fn identity_state(token: &str) -> IdentityState {
    match token {
        "verified" => IdentityState::Verified,
        "tofu" => IdentityState::Tofu,
        "changed" => IdentityState::Changed,
        other => {
            crate::local::app_log::warn(&format!("unknown identity_state: {other}"));
            IdentityState::Tofu
        }
    }
}

/// The security state `state` names; a token not known says nothing is
/// secured yet.
fn security_state(token: &str) -> SecurityState {
    match token {
        "ok" => SecurityState::Ok,
        "pending" => SecurityState::Pending,
        "re_established" => SecurityState::ReEstablished,
        other => {
            crate::local::app_log::warn(&format!("unknown security state: {other}"));
            SecurityState::Pending
        }
    }
}

/// The code of a refusal: its `reason_code`, else its code.
fn code_of(error: &BusError) -> Option<&str> {
    match error {
        BusError::Daemon { reason_code: Some(reason), .. } => Some(reason),
        BusError::Daemon { code, .. } => Some(code),
        _ => None,
    }
}

/// Why a refused `contact.trust.accept_change` did not take effect.
pub(crate) fn accept_failure(error: &BusError) -> AcceptFailure {
    match code_of(error) {
        Some("no_pending_change") => AcceptFailure::AlreadyHandled,
        Some("anchor_mismatch") => AcceptFailure::ChangedAgain,
        _ => AcceptFailure::Failed,
    }
}

impl Translator {
    /// The participant a `contact` identifier names: a participant by id,
    /// else the one whose identity commitment it is, else the identifier
    /// itself (a participant not listed yet).
    pub(super) fn contact_participant(&self, contact: &str) -> ParticipantId {
        let id = ParticipantId::new(contact);
        if self.people.contains_key(&id) {
            return id;
        }
        self.directory
            .identities
            .iter()
            .find(|(_, identity)| identity.commitment.as_deref() == Some(contact))
            .map_or(id, |(principal, _)| ParticipantId::new(principal.as_str()))
    }

    /// Updates what is known of a participant's identity without reporting
    /// the participant again: the identity events carry the change itself.
    fn update_identity(
        &mut self,
        id: &ParticipantId,
        change: impl FnOnce(&mut IdentityRecord),
    ) -> Vec<DomainEvent> {
        let events = self.named(id.as_str());
        if let Some(person) = self.people.get_mut(id) {
            change(&mut person.identity);
            let participant = person.participant(id);
            if self.reported_people.contains_key(id) {
                self.reported_people.insert(id.clone(), participant);
            }
        }
        events
    }

    /// `contact.identity_changed`: the change waits for the user.
    pub(super) fn identity_changed_event(
        &mut self,
        body: &Value,
        now: Timestamp,
    ) -> (Vec<DomainEvent>, Vec<Fetch>) {
        let contact = text(body, "contact");
        let (previous, new) = (text(body, "previous_anchor"), text(body, "new_anchor"));
        if contact.is_empty() || new.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let participant = self.contact_participant(contact);
        let username = Some(text(body, "username")).filter(|u| !u.is_empty()).map(str::to_owned);
        let ai_flag = body["ai_flag"].as_bool().unwrap_or(false);
        let change = IdentityChange {
            previous_anchor: previous.to_owned(),
            new_anchor: new.to_owned(),
            username: username.clone(),
            seen_at: now,
        };
        let mut events = self.update_identity(&participant, |identity| {
            identity.state = IdentityState::Changed;
            identity.pending = Some(change);
            identity.ai_flag = ai_flag;
            identity.contact_ref = Some(contact.to_owned());
        });
        events.push(DomainEvent::IdentityChanged {
            participant: participant.clone(),
            username,
            previous_anchor: previous.to_owned(),
            new_anchor: new.to_owned(),
            ai_flag,
        });
        let follow = vec![Fetch::IdentityStatus {
            participant: participant.to_string(),
            contact: contact.to_owned(),
        }];
        (events, follow)
    }

    /// The answer of `contact.verification.status` for `participant`.
    pub(super) fn identity_status_read(
        &mut self,
        participant: &ParticipantId,
        answer: &Value,
    ) -> Vec<DomainEvent> {
        let Some(token) = answer["identity_state"].as_str() else { return Vec::new() };
        let state = identity_state(token);
        let mut events = self.update_identity(participant, |identity| {
            identity.state = state;
            if !state.is_changed() {
                identity.pending = None;
            }
        });
        events.push(DomainEvent::IdentityStateRead { participant: participant.clone(), state });
        events
    }

    /// The answer of `contact.trust.accept_change` for `participant`.
    pub(super) fn identity_accepted(
        &mut self,
        participant: &ParticipantId,
        accepted: Option<&str>,
        answer: &Value,
    ) -> Vec<DomainEvent> {
        if answer["state"].as_str() != Some("accepted") {
            return vec![DomainEvent::IdentityAcceptFailed {
                participant: participant.clone(),
                reason: AcceptFailure::Failed,
            }];
        }
        // The state is read again by the plan; until then the change is no
        // longer pending.
        // A newer change than the accepted one stays pending.
        let still_pending = self
            .people
            .get(participant)
            .and_then(|p| p.identity.pending.as_ref())
            .is_some_and(|c| Some(c.new_anchor.as_str()) != accepted);
        let mut events = if still_pending {
            Vec::new()
        } else {
            self.update_identity(participant, |identity| {
                identity.pending = None;
                if identity.state.is_changed() {
                    identity.state = IdentityState::Tofu;
                }
            })
        };
        events.push(DomainEvent::IdentityAccepted {
            participant: participant.clone(),
            new_anchor: accepted.map(str::to_owned),
        });
        events
    }

    /// A refused acceptance: the daemon's reason, as an event.
    pub(crate) fn identity_accept_refused(
        &mut self,
        participant: &ParticipantId,
        error: &BusError,
    ) -> Vec<DomainEvent> {
        let reason = accept_failure(error);
        // The change the user looked at is gone, or no longer the latest.
        let mut events = self.update_identity(participant, |identity| {
            if reason == AcceptFailure::AlreadyHandled {
                identity.pending = None;
            }
        });
        events.push(DomainEvent::IdentityAcceptFailed { participant: participant.clone(), reason });
        events
    }

    /// The answer of `conversation.security` for `context`.
    pub(super) fn security_read(&self, context: &ContextId, answer: &Value) -> Vec<DomainEvent> {
        let Some(token) = answer["state"].as_str() else { return Vec::new() };
        let ours = |anchor: &str| {
            !anchor.is_empty()
                && (self.directory.self_commitment.as_deref() == Some(anchor)
                    || self.directory.self_principal.as_deref() == Some(anchor))
        };
        let members = answer["members"].as_array().map_or(&[][..], Vec::as_slice);
        let peer_devices = members.iter().filter(|m| !ours(text(m, "anchor"))).count();
        let any_ai = members.iter().any(|m| m["ai_flag"].as_bool().unwrap_or(false));
        let warning = match answer["warning"]["code"].as_str() {
            Some("fork_recovered") => Some(SecurityWarning::ForkRecovered),
            Some(other) => {
                crate::local::app_log::warn(&format!("unknown security warning: {other}"));
                None
            }
            None => None,
        };
        let security = ConversationSecurity {
            state: security_state(token),
            epoch: answer["epoch"].as_u64().unwrap_or_default(),
            peer_devices: u32::try_from(peer_devices).unwrap_or(u32::MAX),
            any_ai,
            warning,
        };
        vec![DomainEvent::ConversationSecurityRead { context: context.clone(), security }]
    }

    /// A send refused because the contact's identity change is not accepted:
    /// the notice for the composer and the status read that follows. `None`
    /// for any other refusal.
    pub(crate) fn identity_send_blocked(
        &mut self,
        body: &Value,
        message: Option<&str>,
        error: &BusError,
    ) -> Option<(Vec<DomainEvent>, Vec<Fetch>)> {
        if code_of(error) != Some(SEND_PENDING) {
            return None;
        }
        let BusError::Daemon { details, .. } = error else { return None };
        let context = ContextId::new(text(body, "context_id"));
        let named = details.as_deref().map(|d| text(d, "contact")).filter(|c| !c.is_empty());
        let participant = match named {
            Some(contact) => self.contact_participant(contact),
            // No contact named: the other participant of the conversation.
            None => self
                .contexts
                .get(&context)
                .and_then(|c| c.participants.iter().find(|p| !self.is_self(p)).cloned())?,
        };
        let contact = named.map(str::to_owned);
        let mut events = self.update_identity(&participant, |identity| {
            if let Some(contact) = contact {
                identity.contact_ref = Some(contact);
            }
        });
        let moment = message.map(|id| super::conversation::message_moment_id(&context, id));
        events.push(DomainEvent::SendBlockedByIdentity {
            context,
            participant: participant.clone(),
            moment,
        });
        let contact = crate::plan::Known::identity_contact(self, participant.as_str())
            .unwrap_or_else(|| participant.to_string());
        let follow = vec![Fetch::IdentityStatus { participant: participant.to_string(), contact }];
        Some((events, follow))
    }

    /// Whether `participant` is this account.
    fn is_self(&self, participant: &ParticipantId) -> bool {
        self.directory.self_principal.as_deref() == Some(participant.as_str())
    }
}
