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
}

impl SafetyState {
    /// Whether the state is a sticky warning that needs verification.
    pub const fn needs_attention(self) -> bool {
        matches!(self, Self::KeyChanged | Self::Forked)
    }
}

/// A participant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Participant {
    /// Participant id.
    pub id: ParticipantId,
    /// Kind.
    pub kind: ParticipantKind,
    /// Display name.
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
