// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Bus requests of the card runtime (`card.session.*`).
//!
//! The runtime renders a card; the desktop sends it host events and slot
//! replies and draws the batches it returns (see `crate::translate::card_session`).
//! Every method takes the account. Event, slot and submit bodies carry
//! user input and runtime tokens: those fields travel as secrets (spliced
//! into the frame only when it is written, never logged). Only `submit` is
//! a network call on the daemon.

use serde_json::{Value, json};

use crate::bus::BusRequest;

/// The routing label of the card calls.
const SDK_API: &str = "surface";

/// `card.session.open`: a session on the current publication of
/// `surface_ref`, with the app locale and dark flag.
pub fn open(account: &str, surface_ref: &str, locale: &str, dark: bool) -> BusRequest {
    BusRequest::new(
        Some(account),
        SDK_API,
        "card.session.open",
        json!({ "surface_ref": surface_ref, "locale": locale, "dark": dark }),
    )
}

/// `card.session.update`: rebuild the session after `surface.changed`.
pub fn update(account: &str, session: u64) -> BusRequest {
    session_call(account, "card.session.update", session)
}

/// `card.session.present`: a locale or theme change, without a peer update.
pub fn present(account: &str, session: u64, locale: &str, dark: bool) -> BusRequest {
    BusRequest::new(
        Some(account),
        SDK_API,
        "card.session.present",
        json!({ "session": session, "locale": locale, "dark": dark }),
    )
}

/// `card.session.suspend`: drop the VM, keep the drafts.
pub fn suspend(account: &str, session: u64) -> BusRequest {
    session_call(account, "card.session.suspend", session)
}

/// `card.session.resume`: a fresh session identity for the same card.
pub fn resume(account: &str, session: u64) -> BusRequest {
    session_call(account, "card.session.resume", session)
}

/// `card.session.dispose`: release the session.
pub fn dispose(account: &str, session: u64) -> BusRequest {
    session_call(account, "card.session.dispose", session)
}

/// `card.session.submit`: the only networked call; never retried by the
/// desktop after a refusal.
pub fn submit(account: &str, session: u64) -> BusRequest {
    session_call(account, "card.session.submit", session)
}

/// A host event the runtime accepts on a node (spec in the card README).
#[derive(Debug, Clone, PartialEq)]
pub enum CardEvent {
    /// A typed value for a field.
    Input(Value),
    /// A tap on a Button.
    Activate,
    /// A tab selection by index.
    SelectTab(u64),
    /// The host opened a modal.
    ModalOpen,
    /// The host closed a modal.
    ModalClose,
}

impl CardEvent {
    /// The wire form of the event.
    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Self::Input(value) => json!({ "kind": "input", "value": value }),
            Self::Activate => json!({ "kind": "activate" }),
            Self::SelectTab(index) => json!({ "kind": "select_tab", "index": index }),
            Self::ModalOpen => json!({ "kind": "modal_open" }),
            Self::ModalClose => json!({ "kind": "modal_close" }),
        }
    }
}

/// `card.session.event`: one host event on the node addressed by `token`.
/// `token` is the one from the latest batch of this session.
pub fn event(account: &str, session: u64, token: &Value, event: &CardEvent) -> BusRequest {
    BusRequest::new(Some(account), SDK_API, "card.session.event", json!({ "session": session }))
        .with_json("token", token.clone())
        .with_json("event", event.to_value())
}

/// The outcome the host reports for a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotOutcome {
    /// The host confirmed (single tap Confirm, or Open).
    Confirm,
    /// The host denied (approvals).
    Deny,
    /// The host cancelled the sheet.
    Cancel,
    /// The host cannot answer (for example the browser is unavailable).
    Unavailable,
}

impl SlotOutcome {
    const fn wire(self) -> &'static str {
        match self {
            Self::Confirm => "confirm",
            Self::Deny => "deny",
            Self::Cancel => "cancel",
            Self::Unavailable => "unavailable",
        }
    }
}

/// `card.session.slot`: the host's answer to a slot request. The request
/// id and slot id are secret (they fence the reply); the outcome is not.
pub fn slot(
    account: &str,
    session: u64,
    token: &Value,
    slot_id: &str,
    request_id: &str,
    outcome: SlotOutcome,
) -> BusRequest {
    BusRequest::new(
        Some(account),
        SDK_API,
        "card.session.slot",
        json!({ "session": session, "outcome": outcome.wire() }),
    )
    .with_json("token", token.clone())
    .with_secret("slot_id", ramflux_desktop_core::SecretInput::new(slot_id))
    .with_secret("request_id", ramflux_desktop_core::SecretInput::new(request_id))
}

fn session_call(account: &str, method: &str, session: u64) -> BusRequest {
    BusRequest::new(Some(account), SDK_API, method, json!({ "session": session }))
}
