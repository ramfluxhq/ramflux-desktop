// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Participants and contact links.
//!
//! A participant is a principal. What is known about it comes from two
//! lists: the identity directory (display name, kind, safety) and the
//! daemon's friend-link records (`contact.list`, and the record each
//! `contact.*` call answers with), which name a requester and a target
//! principal; the other side of a link is the contact. A pending link is
//! incoming unless this account is its requester. Whatever neither list
//! reports stays unknown: the short id is shown for a missing name, the kind is
//! `Unknown` and safety `Unverified` until the identity directory says
//! otherwise.
//!
//! The daemon resolves contacts at add time and reports, on each
//! `contact.list` row (the link record joined with the contact entry) and
//! as `contact` in the answer to `contact.add`:
//!
//! | field | desktop |
//! |---|---|
//! | `principal_id` | the contact (the link's ids may be root commitments) |
//! | `display_name` (string or null) | the name, when the identity directory has none (a card's name) |
//! | `reachable` (bool) | `reachable`: it holds a verified device to send to |
//! | `verified` (bool) | safety `Verified`, when the identity directory has no row |
//! | `kind_asserted` (kind name or null) | the kind, when the identity directory declares none, marked as self-declared |
//! | `root_source` `lookup` / `card` / `verified` | `root_source`: provisional name lookup, pinned by a card, or compared |
//!
//! A record without these fields leaves them unreported (`reachable` and
//! `root_source` none, the kind not marked as self-declared).

use ramflux_desktop_core::{
    ContactLink, DomainEvent, Participant, ParticipantId, ParticipantKind, RootSource, SafetyState,
};
use serde_json::Value;

use super::{Translator, list, text};
use crate::plan::KnownLink;

/// What the identity directory said about a principal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Facts {
    /// Display name, when an authoritative local source supplied one.
    pub display: Option<String>,
    /// Kind, when declared.
    pub kind: Option<ParticipantKind>,
    /// Verification state.
    pub safety: SafetyState,
    /// Persisted safety number.
    pub safety_number: Option<String>,
    /// Where the pinned root came from.
    pub root_source: Option<RootSource>,
}

/// `root_source` of a contact or identity row.
pub(crate) fn root_source(row: &Value) -> Option<RootSource> {
    match row["root_source"].as_str() {
        Some("lookup") => Some(RootSource::Lookup),
        Some("card") => Some(RootSource::Card),
        Some("verified") => Some(RootSource::Verified),
        _ => None,
    }
}

/// What a friend-link record of a resolving daemon says about the contact.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Resolution {
    /// The name the contact's card gave, when one did.
    pub display: Option<String>,
    /// Whether a verified device to send to is held.
    pub reachable: Option<bool>,
    /// Whether the contact's safety number was verified.
    pub verified: bool,
    /// The kind the contact declares about itself.
    pub kind: Option<ParticipantKind>,
    /// Whether the kind is only the contact's own claim.
    pub kind_asserted: bool,
    /// Where the pinned root came from.
    pub root_source: Option<RootSource>,
}

impl Resolution {
    fn from_record(record: &Value) -> Self {
        let (kind, kind_asserted) = match &record["kind_asserted"] {
            Value::String(name) => (kind_of(name), true),
            Value::Bool(asserted) => (None, *asserted),
            _ => (None, false),
        };
        Self {
            display: crate::directory::field(record, "display_name"),
            reachable: record["reachable"].as_bool(),
            verified: record["verified"].as_bool().unwrap_or(false),
            kind,
            kind_asserted,
            root_source: root_source(record),
        }
    }
}

fn kind_of(name: &str) -> Option<ParticipantKind> {
    match name {
        "human" => Some(ParticipantKind::Human),
        "headless_ai" | "bot" | "agent" | "workflow" => Some(ParticipantKind::Agent),
        _ => None,
    }
}

/// One participant as known from both lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Person {
    /// From the identity directory.
    pub facts: Option<Facts>,
    /// From the friend-link records.
    pub link: ContactLink,
    /// From the friend-link records of a resolving daemon.
    pub resolution: Resolution,
}

impl Person {
    fn participant(&self, id: &ParticipantId) -> Participant {
        let listed = self.facts.is_some();
        let facts = self.facts.clone().unwrap_or_default();
        let resolution = &self.resolution;
        let safety =
            if !listed && resolution.verified { SafetyState::Verified } else { facts.safety };
        let (kind, kind_asserted) = match (facts.kind, resolution.kind) {
            (Some(kind), _) => (kind, resolution.kind_asserted),
            (None, Some(kind)) => (kind, true),
            (None, None) => (ParticipantKind::Unknown, false),
        };
        Participant {
            id: id.clone(),
            kind,
            display: facts
                .display
                .or_else(|| resolution.display.clone())
                .unwrap_or_else(|| id.short()),
            safety,
            contact: self.link == ContactLink::Accepted,
            safety_number: facts.safety_number,
            link: self.link,
            reachable: resolution.reachable,
            kind_asserted,
            root_source: facts.root_source.or(resolution.root_source),
        }
    }
}

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
        let me = self.directory.self_principal.clone().unwrap_or_default();
        let own = |id: &str| {
            !id.is_empty() && (id == me || self.directory.self_commitment.as_deref() == Some(id))
        };
        let (requester, target) = (text(record, "requester_id"), text(record, "target_id"));
        let outgoing = own(requester);
        // The daemon names the contact; the link's ids may be commitments.
        let other = match text(record, "principal_id") {
            "" if outgoing => target,
            "" => requester,
            principal => principal,
        };
        if other.is_empty() || other == me || text(record, "link_id").is_empty() {
            return Vec::new();
        }
        let state = link_state(record, outgoing);
        let id = ParticipantId::new(other);
        self.links.insert(
            id.clone(),
            KnownLink {
                link_id: text(record, "link_id").to_owned(),
                requester_id: requester.to_owned(),
                target_id: target.to_owned(),
                state,
            },
        );
        let person = self.people.entry(id.clone()).or_default();
        person.link = state;
        person.resolution = Resolution::from_record(record);
        let mut events = self.report_person(&id);
        events.extend(self.own_request(record, &id));
        events
    }

    /// The answer to `contact.add` (by id or card): `{contact}`, the
    /// resolved contact, whose link is accepted. With `always`, the contact
    /// is reported even when nothing about it changed: the answer to a card
    /// import says which contact the card named.
    pub(super) fn added_contact(&mut self, answer: &Value, always: bool) -> Vec<DomainEvent> {
        let contact = &answer["contact"];
        let principal = text(contact, "principal_id");
        if principal.is_empty() {
            // An older answer: the link record itself.
            return self.contact_record(answer.get("link").unwrap_or(answer));
        }
        let id = ParticipantId::new(principal);
        let person = self.people.entry(id.clone()).or_default();
        person.link = ContactLink::Accepted;
        person.resolution = Resolution::from_record(contact);
        if let Some(link) = self.links.get_mut(&id) {
            link.state = ContactLink::Accepted;
        }
        let mut events = self.report_person(&id);
        events.extend(self.own_request(contact, &id));
        if !events.is_empty() || !always {
            return events;
        }
        self.reported_people
            .get(&id)
            .map(|p| DomainEvent::ParticipantUpserted(p.clone()))
            .into_iter()
            .collect()
    }

    /// Records what the identity directory says about `id`.
    pub(super) fn set_facts(&mut self, id: &ParticipantId, facts: Facts) -> Vec<DomainEvent> {
        self.people.entry(id.clone()).or_default().facts = Some(facts);
        self.report_person(id)
    }

    /// Makes sure a principal or device the daemon named is a participant
    /// (with nothing known beyond its id).
    pub(super) fn named(&mut self, id: &str) -> Vec<DomainEvent> {
        let id = ParticipantId::new(id);
        if id.as_str().is_empty() || self.people.contains_key(&id) {
            return Vec::new();
        }
        self.people.insert(id.clone(), Person::default());
        self.report_person(&id)
    }

    /// Emits the participant when it differs from what was reported.
    fn report_person(&mut self, id: &ParticipantId) -> Vec<DomainEvent> {
        let Some(person) = self.people.get(id) else { return Vec::new() };
        let participant = person.participant(id);
        if self.reported_people.get(id) == Some(&participant) {
            return Vec::new();
        }
        self.reported_people.insert(id.clone(), participant.clone());
        vec![DomainEvent::ParticipantUpserted(participant)]
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
