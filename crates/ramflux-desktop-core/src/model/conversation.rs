// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Participants and conversation contexts.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ids::{ContextId, ParticipantId};

/// What kind of party a participant is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParticipantKind {
    /// A person.
    Human,
    /// A software agent.
    Agent,
    /// One of the user's own devices.
    Device,
    /// A node acting as a participant.
    Node,
    /// The backend did not report what kind of party this is.
    Unknown,
}

/// What kind of principal a peer states itself to be: the product's
/// principal kinds (`human`, `headless_ai`, `bot`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PeerKind {
    /// A person's account.
    Human,
    /// An AI account without a person attending it.
    HeadlessAi,
    /// A bot.
    Bot,
}

impl PeerKind {
    /// The coarse party kind the pipeline works with.
    pub const fn participant_kind(self) -> ParticipantKind {
        match self {
            Self::Human => ParticipantKind::Human,
            Self::HeadlessAi | Self::Bot => ParticipantKind::Agent,
        }
    }
}

/// How far a participant's kind is proven.
///
/// A verified kind comes from a public profile signed by the same root
/// that signs the peer's device set: it proves what the root claims, not
/// that a person is behind it. An asserted kind is only what the peer (its
/// contact card, or a legacy profile-less device set) says. The safe
/// default for a principal nothing reported about (a name lookup, a legacy
/// identity) is `None`: shown as not verified, its kind unknown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KindClaim {
    /// Nothing reported: the kind is unknown and unverified.
    #[default]
    None,
    /// The peer's own claim; nothing verified it.
    Asserted(PeerKind),
    /// A root-signed public profile states it.
    Verified(PeerKind),
}

impl KindClaim {
    /// The kind claimed or proven, if any.
    pub const fn kind(self) -> Option<PeerKind> {
        match self {
            Self::None => None,
            Self::Asserted(kind) | Self::Verified(kind) => Some(kind),
        }
    }

    /// Whether a root-signed profile states the kind.
    pub const fn is_verified(self) -> bool {
        matches!(self, Self::Verified(_))
    }

    /// Whether the kind is only the peer's own claim.
    pub const fn is_asserted(self) -> bool {
        matches!(self, Self::Asserted(_))
    }
}

/// Warning-only facts about a participant's public name (UTS #39): the
/// name mixes scripts, or looks like another known principal's name. They
/// never say two principals are the same and never block anything; the UI
/// puts the principal id next to such a name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameWarnings {
    /// The name mixes scripts.
    #[serde(default)]
    pub mixed_script: bool,
    /// Another known principal whose name has the same confusable skeleton.
    #[serde(default)]
    pub confusable_with: Option<ParticipantId>,
}

impl NameWarnings {
    /// Whether any warning applies.
    pub const fn any(&self) -> bool {
        self.mixed_script || self.confusable_with.is_some()
    }
}

/// Where the daemon's pinned identity root of a contact came from
/// (`root_source` on contact and identity rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootSource {
    /// A name lookup on the home node: provisional (trusted on first use)
    /// until a card pins the root or safety numbers are compared.
    Lookup,
    /// The contact card they shared: the root it names is pinned.
    Card,
    /// Safety numbers were compared and confirmed.
    Verified,
}

impl RootSource {
    /// Whether the root is only what a name lookup returned.
    pub const fn is_provisional(self) -> bool {
        matches!(self, Self::Lookup)
    }
}

/// Key verification state for a participant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SafetyState {
    /// Keys not verified out of band.
    #[default]
    Unverified,
    /// Safety number verified.
    Verified,
    /// Identity key changed since last seen.
    KeyChanged,
    /// Identity fork detected.
    Forked,
    /// The safety number was verified, but the authority it was verified
    /// against changed since (the daemon's `verification_stale`): the keys
    /// may have changed, so verify again. Sticky like a key change (it
    /// needs attention), said in warning tone as what it is: an earlier
    /// verification that no longer holds.
    VerificationStale,
}

impl SafetyState {
    /// Whether the state is a sticky warning that needs verification.
    pub const fn needs_attention(self) -> bool {
        matches!(self, Self::KeyChanged | Self::Forked | Self::VerificationStale)
    }

    /// Whether an earlier verification no longer holds (verify again).
    pub const fn is_stale(self) -> bool {
        matches!(self, Self::VerificationStale)
    }
}

/// The contact link with a participant, as the backend reports it.
///
/// `None` means no link was reported. A source that reports only the
/// `contact` flag (a scenario) leaves it `None`; [`Participant::link_state`]
/// then reads the flag.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactLink {
    /// No link reported.
    #[default]
    None,
    /// An established contact.
    Accepted,
    /// The participant asked to become a contact; the user has not answered.
    Incoming,
    /// The user asked the participant; no answer yet.
    Outgoing,
    /// The user blocked the participant.
    Blocked,
    /// The contact was removed.
    Removed,
    /// The user rejected the participant's request.
    Rejected,
}

/// A participant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Participant {
    /// Participant id.
    pub id: ParticipantId,
    /// Kind.
    pub kind: ParticipantKind,
    /// Display name: the name a local source gave, else the short form of
    /// the id ([`ParticipantId::short`]).
    pub display: String,
    /// Key verification state.
    #[serde(default)]
    pub safety: SafetyState,
    /// Whether the participant is in the user's contact list.
    #[serde(default)]
    pub contact: bool,
    /// Safety number for out-of-band comparison.
    #[serde(default)]
    pub safety_number: Option<String>,
    /// The contact link, when the source reports one.
    #[serde(default)]
    pub link: ContactLink,
    /// Whether this account can send to the participant now (the daemon
    /// holds a verified device of theirs); `None` when not reported.
    #[serde(default)]
    pub reachable: Option<bool>,
    /// How far `kind` is proven: a root-signed profile, the participant's
    /// own claim, or nothing (the safe default).
    #[serde(default)]
    pub kind_claim: KindClaim,
    /// Warning-only facts about the display name.
    #[serde(default)]
    pub name_warnings: NameWarnings,
    /// Where the daemon's pinned identity root came from; `None` when not
    /// reported.
    #[serde(default)]
    pub root_source: Option<RootSource>,
}

impl Participant {
    /// Whether `display` is a name (not the id or its short form standing
    /// in for a missing one).
    pub fn named(&self) -> bool {
        self.display != self.id.as_str() && self.display != self.id.short()
    }

    /// The contact link, reading the `contact` flag when no link was reported.
    pub fn link_state(&self) -> ContactLink {
        match self.link {
            ContactLink::None if self.contact => ContactLink::Accepted,
            link => link,
        }
    }
}

/// This account's own contact card: what others add to reach it, shared
/// out of band (copied as text or a link, or shown as a QR code).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactCard {
    /// The card as text, exactly as the backend exported it.
    pub text: String,
    /// The same card as a link, when the backend offers one.
    #[serde(default)]
    pub link: Option<String>,
}

/// Context shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextKind {
    /// One-to-one.
    Direct,
    /// Group conversation.
    Group,
    /// Long-lived shared space.
    Space,
}

/// Member role in a group or space: exactly the daemon's roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Role {
    /// Owner.
    Owner,
    /// Administrator.
    Admin,
    /// Regular member.
    Member,
    /// A bot member (an agent admitted to the group).
    Bot,
}

/// Disappearing-message timer of a context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Disappearing {
    /// Messages stay.
    Off,
    /// Messages expire after this many seconds.
    After(u32),
}

/// A conversation context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    /// Context id.
    pub id: ContextId,
    /// Shape.
    pub kind: ContextKind,
    /// Display title.
    pub title: String,
    /// Members in display order.
    #[serde(default)]
    pub participants: Vec<ParticipantId>,
    /// Pinned to the context stack.
    #[serde(default)]
    pub pinned: bool,
    /// Unread moment count reported by the source.
    #[serde(default)]
    pub unread: u32,
    /// Notifications muted.
    #[serde(default)]
    pub muted: bool,
    /// Disappearing-message timer in seconds; `None` keeps messages.
    #[serde(default)]
    pub disappearing_secs: Option<u32>,
    /// Member roles of a group or space; members without an entry are `Member`.
    #[serde(default)]
    pub roles: BTreeMap<ParticipantId, Role>,
    /// Current sender-key epoch of a group or space.
    #[serde(default)]
    pub epoch: Option<u64>,
}

impl Context {
    /// Role of a member; members without an explicit role are `Member`.
    pub fn role_of(&self, participant: &ParticipantId) -> Role {
        self.roles.get(participant).copied().unwrap_or(Role::Member)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claim_names_its_kind_and_how_far_it_is_proven() {
        let none = KindClaim::default();
        assert_eq!(none, KindClaim::None);
        assert_eq!(none.kind(), None);
        assert!(!none.is_verified() && !none.is_asserted());
        let claimed = KindClaim::Asserted(PeerKind::Bot);
        assert_eq!(claimed.kind(), Some(PeerKind::Bot));
        assert!(claimed.is_asserted() && !claimed.is_verified());
        let signed = KindClaim::Verified(PeerKind::HeadlessAi);
        assert_eq!(signed.kind(), Some(PeerKind::HeadlessAi));
        assert!(signed.is_verified() && !signed.is_asserted());
        assert_eq!(PeerKind::Human.participant_kind(), ParticipantKind::Human);
        assert_eq!(PeerKind::HeadlessAi.participant_kind(), ParticipantKind::Agent);
        assert_eq!(PeerKind::Bot.participant_kind(), ParticipantKind::Agent);
    }

    #[test]
    fn a_stale_verification_asks_again_and_needs_attention_like_a_key_change() {
        assert!(SafetyState::VerificationStale.is_stale());
        assert!(SafetyState::VerificationStale.needs_attention());
        assert!(SafetyState::KeyChanged.needs_attention());
        assert!(!SafetyState::Verified.is_stale());
    }

    #[test]
    fn name_warnings_say_whether_any_applies() {
        assert!(!NameWarnings::default().any());
        assert!(NameWarnings { mixed_script: true, confusable_with: None }.any());
        let other = NameWarnings {
            mixed_script: false,
            confusable_with: Some(ParticipantId::new("p-other")),
        };
        assert!(other.any());
    }

    #[test]
    fn a_participant_without_a_claim_is_the_safe_default() {
        let json =
            serde_json::json!({ "id": "p-legacy", "kind": "Unknown", "display": "p-legacy" });
        let legacy: Participant = serde_json::from_value(json).expect("a legacy participant");
        assert_eq!(legacy.kind_claim, KindClaim::None);
        assert_eq!(legacy.safety, SafetyState::Unverified);
        assert!(!legacy.name_warnings.any());
    }
}
