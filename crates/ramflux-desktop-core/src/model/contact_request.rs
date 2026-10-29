// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Contact requests (DESK-F21, contract-first on CONTACT-02).
//!
//! Adding a contact sends the peer a request; until the peer accepts, the
//! two are not contacts. The peer sees the request with the requester's
//! safety number to compare, an optional note, and the messages the
//! requester sent meanwhile, held by the daemon (a "message request") and
//! delivered into the conversation only on accept. The peer accepts,
//! declines or blocks (no further requests or messages from the
//! requester). An account may accept every request automatically.

use serde::{Deserialize, Serialize};

use crate::ids::{ParticipantId, Timestamp};

/// Which side of a request this account is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RequestDirection {
    /// The peer asked this account.
    Incoming,
    /// This account asked the peer.
    Outgoing,
}

/// Where a request is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RequestState {
    /// Not answered yet.
    Pending,
    /// Accepted: the two are contacts.
    Accepted,
    /// Declined: its held messages were dropped.
    Declined,
    /// Blocked: further requests and messages are dropped.
    Blocked,
    /// Expired unanswered.
    Expired,
}

impl RequestState {
    /// Whether the request still waits for an answer.
    pub const fn is_pending(self) -> bool {
        matches!(self, Self::Pending)
    }
}

/// How the account answers what it is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RequestAnswer {
    /// Become contacts; the held messages are delivered.
    Accept,
    /// Refuse; the held messages are dropped.
    Decline,
    /// Refuse and drop any further request or message from the requester.
    Block,
}

/// The account's contact request setting (`contact_requests`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RequestPolicy {
    /// Each request waits for an answer (the default for people).
    Manual,
    /// Every request is accepted at once.
    AutoAccept,
}

/// What the daemon says of its bound on remembered requester identities
/// (`authority_floors` of `contact.requests.list`): each unanswered or
/// recently closed request holds one entry so that an older version of the
/// requester's identity cannot be replayed. At the limit the daemon holds
/// new requests from people it has no entry for until entries expire or are
/// cleared; nothing is dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestFloors {
    /// Entries held now.
    pub retained: u32,
    /// Entries of requests that are closed, which the user may forget.
    pub clearable: u32,
    /// The most entries the daemon holds.
    pub limit: u32,
    /// Whether the limit is reached.
    pub saturated: bool,
}

/// One contact request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactRequest {
    /// The daemon's request id.
    pub id: String,
    /// The other participant (the requester of an incoming request).
    pub peer: ParticipantId,
    /// Which side this account is on.
    pub direction: RequestDirection,
    /// Where it is.
    pub state: RequestState,
    /// The requester's note, as the daemon quoted it.
    #[serde(default)]
    pub note: Option<String>,
    /// The safety number to compare with the peer before accepting.
    #[serde(default)]
    pub safety_number: Option<String>,
    /// Messages the requester sent that the daemon holds until an answer.
    #[serde(default)]
    pub held_messages: u32,
    /// The first held message, as a short preview.
    #[serde(default)]
    pub preview: Option<String>,
    /// When the request expires unanswered.
    #[serde(default)]
    pub expires_at: Option<Timestamp>,
}

impl ContactRequest {
    /// Whether this is a request this account is asked to answer now.
    pub fn awaits_answer(&self) -> bool {
        self.direction == RequestDirection::Incoming && self.state.is_pending()
    }
}
