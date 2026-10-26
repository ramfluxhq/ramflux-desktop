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
//! | `display_name` (string or null) | the name, when the identity directory has none (a card's or profile's name) |
//! | `reachable` (bool) | `reachable`: it holds a verified device to send to |
//! | `verified` (bool) | safety `Verified`, when the identity directory has no row |
//! | `kind_asserted` (`human` / `headless_ai` / `bot`, or null) with `kind_verified` (bool) | the kind claim: `Verified` when a root-signed public profile states it, else `Asserted`; null is `None` (the safe default) |
//! | `display_name_confusable_with` (principal id or null), `mixed_script` (bool) | the name warnings |
//! | `root_source` `lookup` / `card` / `verified` | `root_source`: provisional name lookup, pinned by a card, or compared |
//!
//! A record without these fields leaves them unreported (`reachable` and
//! `root_source` none, the kind claim `None`, no name warning). The
//! identity directory's row wins where both report (its name and kind
//! come from the same profile read); a verified claim wins over an
//! asserted one from either list.
//!
//! An authenticated rename (`contact.changed` with
//! `reason: public_profile_renamed`) becomes a [`MomentKind::NameChange`]
//! in every known context the principal is in, with the old and the new
//! name; the contact and identity rows are read again by the stream.

use ramflux_desktop_core::{
    ContactLink, DomainEvent, KindClaim, Lifecycle, Moment, MomentId, MomentKind,
    NameChangePayload, NameWarnings, Participant, ParticipantId, ParticipantKind, PeerKind,
    RootSource, SafetyState, Timestamp,
};
use serde_json::Value;

use super::{Translator, list, text};
use crate::plan::KnownLink;

/// What the identity directory said about a principal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Facts {
    /// Display name, when an authoritative local source supplied one.
    pub display: Option<String>,
    /// The kind claim.
    pub claim: KindClaim,
    /// Name warnings.
    pub warnings: NameWarnings,
    /// Verification state.
    pub safety: SafetyState,
    /// Persisted safety number.
    pub safety_number: Option<String>,
    /// Where the pinned root came from.
    pub root_source: Option<RootSource>,
}

/// The product's principal kind named by a row (`kind`, `kind_asserted`,
/// `actor_kind`); a workflow principal or anything else is no peer kind.
pub(crate) fn peer_kind(name: &str) -> Option<PeerKind> {
    match name {
        "human" => Some(PeerKind::Human),
        "headless_ai" => Some(PeerKind::HeadlessAi),
        "bot" => Some(PeerKind::Bot),
        _ => None,
    }
}

/// The kind claim of a row: the kind in `field` (a name or null), proven
/// when `kind_verified` is true. A row that names no kind claims nothing,
/// whatever its flag says.
pub(crate) fn kind_claim(row: &Value, field: &str) -> KindClaim {
    match row[field].as_str().and_then(peer_kind) {
        Some(kind) if row["kind_verified"].as_bool().unwrap_or(false) => KindClaim::Verified(kind),
        Some(kind) => KindClaim::Asserted(kind),
        None => KindClaim::None,
    }
}

/// The name warnings of a row (`mixed_script`,
/// `display_name_confusable_with`).
pub(crate) fn name_warnings(row: &Value) -> NameWarnings {
    NameWarnings {
        mixed_script: row["mixed_script"].as_bool().unwrap_or(false),
        confusable_with: row["display_name_confusable_with"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(ParticipantId::new),
    }
}

/// The topic of the daemon's contact events.
pub(crate) const CONTACT_CHANGED: &str = "contact.changed";

/// The `reason` of a `contact.changed` that announces an authenticated
/// rename.
const PUBLIC_PROFILE_RENAMED: &str = "public_profile_renamed";

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
    /// The name the contact's card or profile gave, when one did.
    pub display: Option<String>,
    /// Whether a verified device to send to is held.
    pub reachable: Option<bool>,
    /// Whether the contact's safety number was verified.
    pub verified: bool,
    /// The kind claim.
    pub claim: KindClaim,
    /// Name warnings.
    pub warnings: NameWarnings,
    /// Where the pinned root came from.
    pub root_source: Option<RootSource>,
}

impl Resolution {
    pub(super) fn from_record(record: &Value) -> Self {
        Self {
            display: crate::directory::field(record, "display_name"),
            reachable: record["reachable"].as_bool(),
            verified: record["verified"].as_bool().unwrap_or(false),
            claim: kind_claim(record, "kind_asserted"),
            warnings: name_warnings(record),
            root_source: root_source(record),
        }
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
        // A proven claim wins; else the directory's, else the link's.
        let kind_claim = match (facts.claim, resolution.claim) {
            (KindClaim::Verified(kind), _) | (_, KindClaim::Verified(kind)) => {
                KindClaim::Verified(kind)
            }
            (KindClaim::None, claim) | (claim, _) => claim,
        };
        let kind = kind_claim.kind().map_or(ParticipantKind::Unknown, PeerKind::participant_kind);
        let name_warnings = if facts.warnings.any() || listed {
            facts.warnings
        } else {
            resolution.warnings.clone()
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
            kind_claim,
            name_warnings,
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

    /// A row that names a principal and, like a contact row, says what the
    /// daemon knows of its name and kind (a contact request row): the
    /// participant, with those facts when the row carries them. A row
    /// without them (a count-only event) only names the participant.
    pub(super) fn peer_facts(&mut self, row: &Value) -> Vec<DomainEvent> {
        let id = ParticipantId::new(text(row, "principal_id"));
        if id.as_str().is_empty() {
            return Vec::new();
        }
        if row.get("kind_verified").is_none() && row.get("display_name").is_none() {
            return self.named(id.as_str());
        }
        let person = self.people.entry(id.clone()).or_default();
        let link = person.resolution.reachable;
        let (verified, root_source) = (person.resolution.verified, person.resolution.root_source);
        let mut resolution = Resolution::from_record(row);
        // A request row says nothing about reachability, verification or
        // the root: what the contact row said stays.
        resolution.reachable = link;
        resolution.verified = verified;
        resolution.root_source = root_source;
        person.resolution = resolution;
        self.report_person(&id)
    }

    /// The moments a `contact.changed` event adds by itself: an
    /// authenticated rename, as a name change in every known context the
    /// principal is in. Any other contact event adds nothing (the reads it
    /// calls for show it).
    pub(super) fn contact_changed(&mut self, body: &Value, now: Timestamp) -> Vec<DomainEvent> {
        if text(body, "reason") != PUBLIC_PROFILE_RENAMED {
            return Vec::new();
        }
        let principal = ParticipantId::new(text(body, "principal_id"));
        let (old_name, new_name) = (text(body, "old_display_name"), text(body, "new_display_name"));
        if principal.as_str().is_empty() || new_name.is_empty() || old_name == new_name {
            return Vec::new();
        }
        let contexts: Vec<_> = self
            .contexts
            .values()
            .filter(|context| context.participants.contains(&principal))
            .map(|context| context.id.clone())
            .collect();
        let mut events = self.named(principal.as_str());
        for context in contexts {
            let digest = super::digest(&MomentKind::NameChange(NameChangePayload {
                participant_id: principal.clone(),
                old_name: old_name.to_owned(),
                new_name: new_name.to_owned(),
            }));
            let moment = Moment {
                id: MomentId::new(format!("name-change:{context}:{principal}:{digest:x}")),
                context_id: context,
                origin: principal.clone(),
                at: now,
                kind: MomentKind::NameChange(NameChangePayload {
                    participant_id: principal.clone(),
                    old_name: old_name.to_owned(),
                    new_name: new_name.to_owned(),
                }),
                lifecycle: Lifecycle::Done,
            };
            events.extend(self.report_moment(moment));
        }
        events
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
