// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Contact cards: adding a contact from the card they shared, and
//! exporting this account's own card.
//!
//! A contact card is the daemon's signed record of an identity (principal,
//! root commitment, home node and its key fingerprint, device hints, display
//! name), exchanged out of band as text, a link or a QR code. The desktop
//! never reads or builds one: it passes the text the user pasted or
//! imported to the daemon, which checks the signature, pins the root
//! commitment the card names and resolves the identity through its
//! verification chain, and it shows the exported text as it came.
//!
//! - Importing a card is `contact.add` `{card}` (the desktop's
//!   `contact.card.import`): the daemon verifies the card's root
//!   commitment, signature and expiry, pins the root it names
//!   (`root_source: card`) and answers `{contact}` like an add by id. The
//!   identity directory and the contact list are read again, so the
//!   contact's devices (and with them a route for a first message and the
//!   safety number) show at once.
//! - When the card names another root than a provisional name lookup
//!   cached, the daemon refuses with
//!   `root_replacement_confirmation_required` and both roots in `details`;
//!   after the user compared them, the same card goes again with
//!   `replace_provisional_root` (the old root, a compare-and-replace guard).
//! - `contact.card.export` `{}`: answers `{card, link}`. A card answered as
//!   a JSON object is shown as its compact JSON text.
//!
//! Both need the account's home-node profile on the daemon: a card is
//! accepted only for the account's own home node.

use ramflux_desktop_core::ContactCard;
use serde_json::{Value, json};

use super::fetch::{Fetch, Paged};
use super::{Plan, refuse_invalid, text};
use crate::bus::BusRequest;

/// The reads after a contact is added: the contact list (the link) and the
/// identity directory (their devices, commitment and safety state).
pub(super) fn contact_refresh() -> Vec<Fetch> {
    vec![Fetch::ContactList, Fetch::first(Paged::Identities)]
}

/// Plans `contact.card.import`.
pub(super) fn import(acct: Option<&str>, body: &Value) -> Plan {
    let card = text(body, "card");
    if card.is_empty() {
        return refuse_invalid("no contact card given");
    }
    let mut add = json!({ "card": card });
    let replace = text(body, "replace_provisional_root");
    if !replace.is_empty() {
        add["replace_provisional_root"] = json!(replace);
    }
    Plan::Call {
        request: BusRequest::new(acct, "contact", "contact.add", add),
        refresh: contact_refresh(),
    }
}

/// Plans `contact.card.export`.
pub(super) fn export(acct: Option<&str>) -> Plan {
    Plan::Call {
        request: BusRequest::new(acct, "contact", "contact.card.export", json!({})),
        refresh: Vec::new(),
    }
}

/// Reads the exported card from the daemon's answer.
pub fn exported_card(answer: &Value) -> Option<ContactCard> {
    let text = match &answer["card"] {
        Value::String(card) if !card.trim().is_empty() => card.clone(),
        card @ Value::Object(_) => serde_json::to_string(card).ok()?,
        _ => return None,
    };
    let link = answer["link"].as_str().filter(|l| !l.trim().is_empty()).map(str::to_owned);
    Some(ContactCard { text, link })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_cards_are_read_as_text() {
        let card = exported_card(&json!({ "card": "rf-card", "link": "ramflux:contact/x" }));
        assert_eq!(
            card,
            Some(ContactCard { text: "rf-card".into(), link: Some("ramflux:contact/x".into()) })
        );
        let object = exported_card(&json!({ "card": { "v": 1 }, "link": "" }));
        assert_eq!(object, Some(ContactCard { text: r#"{"v":1}"#.into(), link: None }));
        assert_eq!(exported_card(&json!({ "card": " " })), None);
        assert_eq!(exported_card(&json!({})), None);
    }
}
