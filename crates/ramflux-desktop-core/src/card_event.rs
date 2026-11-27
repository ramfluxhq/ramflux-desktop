// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Card runtime facts the store applies (`card.session.*` replies, as the
//! SDK reads them) and the status the desktop overlays on a card.
//!
//! The runtime owns the card: its tree, drafts and budgets. The store holds
//! the last tree it was sent, the session that tree belongs to, and what the
//! desktop must still do about it (see [`crate::store::cards`]). Link and
//! slot fields are secret (they fence the reply); their `Debug` output is
//! redacted and they never reach a log.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::SurfaceId;
use crate::model::CardBatch;

/// What the desktop overlays on a card beyond the runtime's tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CardStatus {
    /// Interactive, as the tree says.
    Live,
    /// A submit reached the daemon; the card waits for its signed update.
    Consumed,
    /// The host's approval verdict is recorded; the card waits for the
    /// signed update (the decision itself is not made here).
    AwaitingConfirmation,
    /// The held tree no longer follows the runtime's session: re-read it.
    Stale,
    /// A button was consumed for a submit the desktop must send now.
    PendingSubmit,
    /// The runtime does not allow drawing the card: fallback text only.
    Fallback,
}

/// A slot outcome the host reports (`card.session.slot`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CardSlotOutcome {
    /// Confirm (single tap Confirm, or Open).
    Confirm,
    /// Deny (approvals).
    Deny,
    /// Cancel the sheet.
    Cancel,
    /// The host cannot answer.
    Unavailable,
}

/// A link the runtime asked the host to show: the sheet names `url`; the
/// browser opens only after the slot answers `opened`.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct CardLink {
    /// Slot id to answer (secret).
    pub slot_id: String,
    /// Request id to answer (secret).
    pub request_id: String,
    /// The exact URL to show.
    pub url: String,
    /// Token that fences the slot answer (secret).
    pub token: Value,
}

impl fmt::Debug for CardLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CardLink")
            .field("url", &self.url)
            .field("slot_id", &"<redacted>")
            .field("request_id", &"<redacted>")
            .field("token", &"<redacted>")
            .finish()
    }
}

/// The host part of an `https` URL, or `None` when the URL is not an https
/// URL with a plain host (no userinfo). A URL with any character outside
/// printable ASCII (whitespace, controls, and every non-ASCII character,
/// format characters such as bidirectional overrides included) is refused,
/// never cleaned: the sheet shows exactly the URL that would open. A link
/// sheet shows this host on its own line; a link the host refuses is never
/// opened.
#[must_use]
pub fn https_host(url: &str) -> Option<&str> {
    if !url.bytes().all(|b| b.is_ascii_graphic()) {
        return None;
    }
    let scheme = url.get(..8)?;
    if !scheme.eq_ignore_ascii_case("https://") {
        return None;
    }
    let after = &url[8..];
    let authority = after.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => host,
        Some(_) => return None,
        None => authority,
    };
    let valid = !host.is_empty()
        && host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    valid.then_some(host)
}

/// The receipt of a submit (what the daemon accepted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardReceipt {
    /// Message id the daemon assigned.
    pub message_id: String,
    /// Surface version that was submitted.
    pub surface_version: u64,
    /// Surface hash that was submitted.
    pub surface_hash: String,
}

/// The refusal reason the desktop gives an event taken on a card version the
/// card no longer holds: the user reviews the card again first.
pub const CARD_CHANGED_REASON: &str = "card_changed";

/// The reason key of a daemon without the card runtime.
pub const CARD_RUNTIME_GAP: &str = "backend.gap.surfaces";

/// The bus methods the card runtime serves (gapped together when it is missing).
pub const CARD_METHODS: [&str; 9] = [
    "card.session.open",
    "card.session.update",
    "card.session.present",
    "card.session.event",
    "card.session.slot",
    "card.session.submit",
    "card.session.suspend",
    "card.session.resume",
    "card.session.dispose",
];

/// A card runtime fact about one surface (its `surface_ref`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CardEvent {
    /// A session opened for the card: its full tree.
    Opened {
        /// Surface.
        surface_id: SurfaceId,
        /// The new session.
        session: u64,
        /// The open batch (the whole tree).
        batch: Box<CardBatch>,
    },
    /// A suspended card resumed under a fresh session: its full tree.
    Resumed {
        /// Surface.
        surface_id: SurfaceId,
        /// The fresh session.
        session: u64,
        /// The resume batch (the whole tree).
        batch: Box<CardBatch>,
    },
    /// A batch for the held session: a signed update, an edit or a locale
    /// change. `draft_conflict` says the runtime discarded the author's
    /// draft in favour of the signed update. `own_reply` says the batch
    /// answers an event this desktop sent (an edit or a tap); any other
    /// batch is an outside change.
    Batch {
        /// Surface.
        surface_id: SurfaceId,
        /// The batch to apply to the held tree.
        batch: Box<CardBatch>,
        /// The runtime discarded a local draft.
        draft_conflict: bool,
        /// The batch answers an event this desktop sent.
        #[serde(default)]
        own_reply: bool,
    },
    /// The surface changed on the daemon, the runtime superseded the
    /// session, or the held tree no longer follows it: the card is read
    /// again (`card.session.update`).
    Stale {
        /// Surface.
        surface_id: SurfaceId,
    },
    /// The runtime cannot draw the card: fallback text only.
    Fallback {
        /// Surface.
        surface_id: SurfaceId,
    },
    /// A host event call (`card.session.event`) on the card has completed,
    /// whatever its reply or failure. Always the last fact of that call.
    EventSettled {
        /// Surface.
        surface_id: SurfaceId,
    },
    /// The submit did not reach the daemon or its answer is unknown (a
    /// timeout or a broken connection). The button stays consumed; the user
    /// may retry the submit, it is never retried by itself.
    SubmitFailed {
        /// Surface.
        surface_id: SurfaceId,
    },
    /// A tap consumed a Button; the submit must follow.
    PendingSubmit {
        /// Surface.
        surface_id: SurfaceId,
    },
    /// The submit reached the daemon.
    Submitted {
        /// Surface.
        surface_id: SurfaceId,
        /// What was accepted.
        receipt: CardReceipt,
    },
    /// The session was suspended (its tree stays).
    Suspended {
        /// Surface.
        surface_id: SurfaceId,
    },
    /// The session was released.
    Disposed {
        /// Surface.
        surface_id: SurfaceId,
    },
    /// A link activation needs the host's sheet.
    OpenUrl {
        /// Surface.
        surface_id: SurfaceId,
        /// The link to show.
        link: CardLink,
    },
    /// The host confirmed the link sheet: the browser may open `url` now.
    LinkOpened {
        /// Surface.
        surface_id: SurfaceId,
        /// The URL the sheet showed (validated https by the runtime).
        url: String,
    },
    /// The link sheet is closed (cancelled or unavailable, or after opened).
    LinkClosed {
        /// Surface.
        surface_id: SurfaceId,
    },
    /// The host's approval verdict was recorded; not yet an author effect.
    ApprovalVerdict {
        /// Surface.
        surface_id: SurfaceId,
        /// Confirm or deny.
        outcome: CardSlotOutcome,
    },
    /// A call was refused, by its typed reason.
    Refused {
        /// Surface.
        surface_id: SurfaceId,
        /// The reason key (`card_session_*`, or `stale_dropped`).
        reason: String,
    },
}

impl CardEvent {
    /// The surface the fact is about.
    pub fn surface_id(&self) -> &SurfaceId {
        match self {
            Self::Opened { surface_id, .. }
            | Self::Resumed { surface_id, .. }
            | Self::Batch { surface_id, .. }
            | Self::Stale { surface_id }
            | Self::Fallback { surface_id }
            | Self::EventSettled { surface_id }
            | Self::SubmitFailed { surface_id }
            | Self::PendingSubmit { surface_id }
            | Self::Submitted { surface_id, .. }
            | Self::Suspended { surface_id }
            | Self::Disposed { surface_id }
            | Self::OpenUrl { surface_id, .. }
            | Self::LinkOpened { surface_id, .. }
            | Self::LinkClosed { surface_id }
            | Self::ApprovalVerdict { surface_id, .. }
            | Self::Refused { surface_id, .. } => surface_id,
        }
    }
}
