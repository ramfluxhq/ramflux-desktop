// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Participants and contact links.
//!
//! The daemon's friend-link records (`contact.list`, and the record each
//! `contact.*` call answers with) name a requester and a target principal;
//! the other side of a link is the contact. The link state maps to
//! [`ContactLink`]: a pending link is incoming unless this account is its
//! requester. The daemon reports no display names, kinds or safety state:
//! the id is shown, the kind stays `Human` and safety `Unverified`.

use ramflux_desktop_core::{
    ContactLink, DomainEvent, Participant, ParticipantId, ParticipantKind, SafetyState,
};
use serde_json::Value;

use super::{Translator, list, text};
use crate::plan::KnownLink;

impl Translator {
    pub(super) fn contact_list(&mut self, value: &Value) -> Vec<DomainEvent> {
        let mut events = Vec::new();
        for record in list(value, "contacts") {
            events.extend(self.contact_record(record));
        }
        events
    }

    /// One friend-link record. Later records of the same contact win (the
    /// daemon lists links oldest first).
    pub(super) fn contact_record(&mut self, record: &Value) -> Vec<DomainEvent> {
        let me = self.principal.clone().unwrap_or_default();
        let (requester, target) = (text(record, "requester_id"), text(record, "target_id"));
        let outgoing = !me.is_empty() && requester == me;
        let other = if outgoing { target } else { requester };
        if other.is_empty() || other == me || text(record, "link_id").is_empty() {
            return Vec::new();
        }
        let state = link_state(record, outgoing);
        self.links.insert(
            ParticipantId::new(other),
            KnownLink {
                link_id: text(record, "link_id").to_owned(),
                requester_id: requester.to_owned(),
                target_id: target.to_owned(),
                state,
            },
        );
        self.linked_participant(other, state)
    }

    /// Upserts a participant the daemon named outside a contact link (a
    /// sender, a member, this device), once.
    pub(super) fn participant(&mut self, id: &str, display: &str) -> Vec<DomainEvent> {
        let id = ParticipantId::new(id);
        if self.participants.contains_key(&id) {
            return Vec::new();
        }
        self.participants.insert(id.clone(), ContactLink::None);
        vec![upserted(id, display, ContactLink::None)]
    }

    /// Upserts a contact whose link state changed.
    fn linked_participant(&mut self, id: &str, link: ContactLink) -> Vec<DomainEvent> {
        let participant = ParticipantId::new(id);
        if self.participants.get(&participant) == Some(&link) {
            return Vec::new();
        }
        self.participants.insert(participant.clone(), link);
        vec![upserted(participant, id, link)]
    }

    /// The link this account has with `participant`, as last listed.
    pub(crate) fn known_link(&self, participant: &str) -> Option<&KnownLink> {
        self.links.get(&ParticipantId::new(participant))
    }
}

fn link_state(record: &Value, outgoing: bool) -> ContactLink {
    if record["blocked"].as_bool().unwrap_or(false) {
        return ContactLink::Blocked;
    }
    match text(record, "state") {
        "accepted" => ContactLink::Accepted,
        "pending" if outgoing => ContactLink::Outgoing,
        "pending" => ContactLink::Incoming,
        "blocked" => ContactLink::Blocked,
        "removed" => ContactLink::Removed,
        "rejected" => ContactLink::Rejected,
        _ => ContactLink::None,
    }
}

fn upserted(id: ParticipantId, display: &str, link: ContactLink) -> DomainEvent {
    DomainEvent::ParticipantUpserted(Participant {
        id,
        kind: ParticipantKind::Human,
        display: display.to_owned(),
        safety: SafetyState::Unverified,
        contact: link == ContactLink::Accepted,
        safety_number: None,
        link,
    })
}
