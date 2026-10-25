// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Contact requests (DESK-F21, contract-first on CONTACT-02).
//!
//! The daemon lists the pending requests and the account's setting with
//! `contact.requests.list` → `{requests: [row], contact_requests:
//! "manual" | "auto_accept"}`. A row: `request_id`, `principal_id` (the
//! peer), `direction` (`inbound` / `outbound`; `incoming` / `outgoing` are
//! read too), `state` (`pending`, `accepted`, `declined`, `blocked`,
//! `expired`), `note` (the requester's note, quoted by the daemon),
//! `safety_number`, `message_count` (held messages), `preview` (the first
//! held message), `expires_at` (Unix seconds).
//!
//! Each change comes as an event with a row (at least `request_id`,
//! `principal_id`, `state`, `direction`): `contact.requested` (a new
//! incoming request), `contact.accepted`, `contact.declined`,
//! `contact.blocked`, `contact.request.expired`. An event is reported at
//! once and calls for the request list and the contacts again; an accept
//! also calls for the conversations (the held messages are delivered).
//!
//! This account's own request to a peer shows on the peer's contact row
//! (`request_state` in `contact.list` and in the answer to `contact.add`).

use ramflux_desktop_core::{
    ContactRequest, DomainEvent, ParticipantId, RequestDirection, RequestPolicy, RequestState,
};
use serde_json::Value;

use super::{Translated, Translator, list, seconds, text};
use crate::plan::fetch::{Fetch, Paged};

/// The request topics of the event stream.
pub const TOPICS: [&str; 5] = [
    "contact.requested",
    "contact.accepted",
    "contact.declined",
    "contact.blocked",
    "contact.request.expired",
];

fn state(name: &str) -> Option<RequestState> {
    Some(match name {
        "pending" => RequestState::Pending,
        "accepted" => RequestState::Accepted,
        "declined" | "rejected" => RequestState::Declined,
        "blocked" => RequestState::Blocked,
        "expired" => RequestState::Expired,
        _ => return None,
    })
}

fn direction(name: &str) -> Option<RequestDirection> {
    match name {
        "inbound" | "incoming" => Some(RequestDirection::Incoming),
        "outbound" | "outgoing" => Some(RequestDirection::Outgoing),
        _ => None,
    }
}

/// The policy a `contact_requests` value names.
pub fn policy(name: &str) -> Option<RequestPolicy> {
    match name {
        "manual" => Some(RequestPolicy::Manual),
        "auto_accept" => Some(RequestPolicy::AutoAccept),
        _ => None,
    }
}

fn optional_text(row: &Value, field: &str) -> Option<String> {
    row.get(field).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned)
}

/// The request a row describes; `None` when it lacks what a request needs
/// (nothing is guessed).
pub fn request_of(row: &Value, default_state: Option<RequestState>) -> Option<ContactRequest> {
    let peer = text(row, "principal_id");
    let id = text(row, "request_id");
    if peer.is_empty() || id.is_empty() {
        return None;
    }
    Some(ContactRequest {
        id: id.to_owned(),
        peer: ParticipantId::new(peer),
        direction: direction(text(row, "direction"))?,
        state: state(text(row, "state")).or(default_state)?,
        note: optional_text(row, "note"),
        safety_number: optional_text(row, "safety_number"),
        held_messages: row["message_count"]
            .as_u64()
            .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX)),
        preview: optional_text(row, "preview"),
        expires_at: row.get("expires_at").and_then(seconds),
    })
}

/// The state a request event reports, when the row does not say.
fn topic_state(topic: &str) -> Option<RequestState> {
    Some(match topic {
        "contact.requested" => RequestState::Pending,
        "contact.accepted" => RequestState::Accepted,
        "contact.declined" => RequestState::Declined,
        "contact.blocked" => RequestState::Blocked,
        "contact.request.expired" => RequestState::Expired,
        _ => return None,
    })
}

impl Translator {
    /// A `contact.requests.list` answer: the pending requests (both ways)
    /// and the account's setting.
    pub(super) fn contact_request_list(&mut self, value: &Value) -> Vec<DomainEvent> {
        let requests: Vec<ContactRequest> =
            list(value, "requests").iter().filter_map(|row| request_of(row, None)).collect();
        let mut events: Vec<DomainEvent> =
            requests.iter().flat_map(|r| self.named(r.peer.as_str())).collect();
        let (incoming, outgoing): (Vec<_>, Vec<_>) =
            requests.into_iter().partition(|r| r.direction == RequestDirection::Incoming);
        events.push(DomainEvent::ContactRequestsListed(incoming));
        for request in outgoing {
            self.outgoing_requests.insert(request.peer.clone(), request.clone());
            events.push(DomainEvent::ContactRequestUpserted(request));
        }
        if let Some(policy) = policy(text(value, "contact_requests")) {
            events.push(DomainEvent::ContactRequestPolicyUpdated(policy));
        }
        events
    }

    /// One request event: the request as the row says, and the reads it
    /// calls for.
    pub fn contact_request_event(&mut self, topic: &str, body: &Value) -> Translated {
        let Some(request) = request_of(body, topic_state(topic)) else {
            return Translated { events: Vec::new(), follow: vec![Fetch::ContactRequests] };
        };
        let accepted = request.state == RequestState::Accepted;
        let mut events = self.named(request.peer.as_str());
        if request.direction == RequestDirection::Outgoing {
            self.outgoing_requests.insert(request.peer.clone(), request.clone());
        }
        events.push(DomainEvent::ContactRequestUpserted(request));
        let mut follow =
            vec![Fetch::ContactRequests, Fetch::ContactList, Fetch::first(Paged::Identities)];
        if accepted {
            follow.push(Fetch::first(Paged::Conversations));
        }
        Translated { events, follow }
    }

    /// This account's own request to the contact a `contact.list` row or a
    /// `contact.add` answer names (`request_state`), if it reports one.
    pub(super) fn own_request(&mut self, row: &Value, peer: &ParticipantId) -> Option<DomainEvent> {
        let state = state(text(row, "request_state"))?;
        let known = self.outgoing_requests.get(peer);
        if known.is_some_and(|r| r.state == state) {
            return None;
        }
        let id = optional_text(row, "request_id")
            .or_else(|| known.map(|r| r.id.clone()))
            .unwrap_or_default();
        let request = ContactRequest {
            id,
            peer: peer.clone(),
            direction: RequestDirection::Outgoing,
            state,
            note: None,
            safety_number: None,
            held_messages: 0,
            preview: None,
            expires_at: None,
        };
        self.outgoing_requests.insert(peer.clone(), request.clone());
        Some(DomainEvent::ContactRequestUpserted(request))
    }
}
