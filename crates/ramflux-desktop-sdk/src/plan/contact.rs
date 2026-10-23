// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Contact effects to the daemon's `contact.*` requests.
//!
//! Bodies are built from the product SDK's own request types
//! (`LocalBusContactAddRequest`, `LocalBusContactLinkRequest`,
//! `LocalBusContactRemoveRequest`), so their shape is the daemon's. The
//! desktop names a contact by participant (principal) id; the daemon names
//! a link by link id, which the source knows from `contact.list`.
//!
//! - `contact.add`: this account (requester) to the typed principal
//!   (target). The link id is derived from both ids, so adding the same
//!   contact again re-establishes the same link.
//! - `contact.accept`: the local form (requester, target of the pending
//!   link). The federated form, which also notifies the requester, needs the
//!   requester's device route; the daemon does not list it.
//! - `contact.reject`, `contact.block`, `contact.unblock`: the link id.
//! - `contact.remove`: the link id and the reference CLI's default scope `me`.
//! - `safety.verify`: `contact.verify` with the contact's identity
//!   commitment from `identity.directory`; the daemon fetches both device
//!   sets from the node, computes the safety number and marks it verified.
//!
//! A contact request (`contact.request`) is the federated form only: it
//! needs a federation route to the contact's node, which no directory lists
//! for a contact, so it stays unsupported.

use ramflux_desktop_core::{ContactLink, methods};
use ramflux_sdk::{
    LocalBusContactAddRequest, LocalBusContactLinkRequest, LocalBusContactRemoveRequest,
    LocalBusContactSafetyRequest,
};
use serde_json::Value;

use super::fetch::{Fetch, Paged};
use super::{Known, Plan, refuse, refuse_invalid, text};
use crate::bus::BusRequest;
use crate::translate::reason::key;

/// Domain tag of desktop-derived link ids.
const LINK_ID_DOMAIN: &str = "ramflux.desktop.contact_link.v1";

/// Scope of `contact.remove` (the reference CLI's default).
pub const REMOVE_SCOPE: &str = "me";

/// A contact link the daemon listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownLink {
    /// The daemon's link id.
    pub link_id: String,
    /// Requesting principal.
    pub requester_id: String,
    /// Target principal.
    pub target_id: String,
    /// State.
    pub state: ContactLink,
}

/// The link id the desktop uses for a link from `requester` to `target`.
pub fn derived_link_id(requester: &str, target: &str) -> String {
    let digest =
        ramflux_protocol::hash_hex(LINK_ID_DOMAIN, format!("{requester}\n{target}").as_bytes());
    format!("link-{}", &digest[..32])
}

/// Plans a `contact.*` effect for account `acct`.
pub(super) fn contact_call(
    acct: Option<&str>,
    method: &str,
    body: &Value,
    known: &impl Known,
) -> Plan {
    // A moment-based answer names the requester as `from`.
    let participant = match text(body, "participant_id") {
        "" => text(body, "from"),
        id => id,
    };
    if participant.is_empty() {
        return refuse_invalid("no contact named");
    }
    let request = |daemon: &str, value: Result<Value, serde_json::Error>| match value {
        Ok(value) => Plan::Call {
            request: BusRequest::new(acct, "contact", daemon, value),
            refresh: vec![Fetch::ContactList],
        },
        Err(error) => refuse_invalid(&error.to_string()),
    };
    if method == methods::CONTACT_ADD {
        let Some(me) = known.self_principal() else {
            return refuse_invalid("this account's identity is not known yet");
        };
        if participant == me {
            return refuse_invalid("that is this account's own id");
        }
        let link_id = known
            .contact_link(participant)
            .filter(|link| link.requester_id == me)
            .map_or_else(|| derived_link_id(me, participant), |link| link.link_id.clone());
        let add = LocalBusContactAddRequest {
            link_id,
            requester_id: me.to_owned(),
            target_id: participant.to_owned(),
        };
        return request("contact.add", serde_json::to_value(add));
    }
    let Some(link) = known.contact_link(participant) else {
        return refuse_invalid("the daemon lists no link with this contact");
    };
    let link_request = || LocalBusContactLinkRequest { link_id: link.link_id.clone() };
    match method {
        methods::CONTACT_ACCEPT => {
            if link.state != ContactLink::Incoming {
                return refuse_invalid("there is no pending request from this contact");
            }
            let accept = LocalBusContactAddRequest {
                link_id: link.link_id.clone(),
                requester_id: link.requester_id.clone(),
                target_id: link.target_id.clone(),
            };
            request("contact.accept", serde_json::to_value(accept))
        }
        methods::CONTACT_REJECT => request("contact.reject", serde_json::to_value(link_request())),
        methods::CONTACT_BLOCK => request("contact.block", serde_json::to_value(link_request())),
        methods::CONTACT_UNBLOCK => {
            request("contact.unblock", serde_json::to_value(link_request()))
        }
        methods::CONTACT_REMOVE => {
            let remove = LocalBusContactRemoveRequest {
                link_id: link.link_id.clone(),
                scope: REMOVE_SCOPE.to_owned(),
            };
            request("contact.remove", serde_json::to_value(remove))
        }
        _ => refuse_invalid("no daemon counterpart"),
    }
}

/// Plans `safety.verify` for account `acct`.
pub(super) fn verify(acct: Option<&str>, body: &Value, known: &impl Known) -> Plan {
    let commitment = known
        .directory()
        .and_then(|d| d.identities.get(text(body, "participant_id")))
        .and_then(|identity| identity.commitment.clone());
    let Some(commitment) = commitment else { return refuse(key::NO_COMMITMENT) };
    let verify = LocalBusContactSafetyRequest { contact_identity_commitment: commitment };
    match serde_json::to_value(verify) {
        Ok(value) => Plan::Call {
            request: BusRequest::new(acct, "contact", "contact.verify", value),
            refresh: vec![Fetch::first(Paged::Identities)],
        },
        Err(error) => refuse_invalid(&error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_link_ids_are_stable_and_directional() {
        let id = derived_link_id("principal-me", "principal-ana");
        assert_eq!(id, derived_link_id("principal-me", "principal-ana"));
        assert_ne!(id, derived_link_id("principal-ana", "principal-me"));
        assert!(id.starts_with("link-") && id.len() == 37);
    }
}
