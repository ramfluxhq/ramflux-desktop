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
//! - `contact.add` `{principal_id}` (exactly that field: the daemon refuses
//!   any other): the daemon looks the principal up through the account's
//!   authenticated home-node session, verifies its device manifest and
//!   caches it with an accepted link, so the contact is reachable at once.
//!   A name lookup is provisional (`root_source: lookup`, trust on first
//!   use) until a card pins the root or safety numbers are compared. The
//!   daemon derives the link id.
//! - `contact.accept`: the local form (requester, target of the pending
//!   link). The federated form, which also notifies the requester, needs the
//!   requester's device route; the daemon does not list it.
//! - Adding and accepting read the identity directory again with the
//!   contact list: the contact's devices and safety state follow the link.
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
            refresh: if matches!(daemon, "contact.add" | "contact.accept") {
                super::contact_card::contact_refresh()
            } else {
                vec![Fetch::ContactList]
            },
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
        return request("contact.add", Ok(serde_json::json!({ "principal_id": participant })));
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

/// Plans an answer to an incoming contact request, or the request setting
/// (CONTACT-02, contract-first): `contact.request.accept|decline|block`
/// become the daemon's `contact.accept|decline|block {request_id}`; the
/// setting is `contact.requests.policy.set {contact_requests}`. An answer
/// reads the requests and contacts again, and an accept the conversations
/// too (the held messages are delivered into the conversation).
pub(super) fn request_call(acct: Option<&str>, method: &str, body: &Value) -> Plan {
    if method == methods::CONTACT_REQUESTS_POLICY {
        let policy = text(body, "contact_requests");
        if crate::translate::contact_requests::policy(policy).is_none() {
            return refuse_invalid("unknown contact request setting");
        }
        let mut daemon_body = serde_json::json!({ "contact_requests": policy });
        // With the setting, the daemon forgets the entries of closed requests
        // (`cleared_authority_floors` says how many); the setting stays.
        if body.get("clear_retired_authority_floors").and_then(Value::as_bool) == Some(true) {
            daemon_body["clear_retired_authority_floors"] = serde_json::json!(true);
        }
        return Plan::Call {
            request: BusRequest::new(acct, "contact", "contact.requests.policy.set", daemon_body),
            refresh: vec![Fetch::ContactRequests],
        };
    }
    let request_id = text(body, "request_id");
    if request_id.is_empty() {
        return refuse_invalid("no contact request named");
    }
    let (daemon, accept) = match method {
        methods::CONTACT_REQUEST_ACCEPT => ("contact.accept", true),
        methods::CONTACT_REQUEST_DECLINE => ("contact.decline", false),
        methods::CONTACT_REQUEST_BLOCK => ("contact.block", false),
        _ => return refuse_invalid("no daemon counterpart"),
    };
    let mut refresh = vec![Fetch::ContactRequests];
    refresh.extend(super::contact_card::contact_refresh());
    if accept {
        refresh.push(Fetch::first(Paged::Conversations));
    }
    Plan::Call {
        request: BusRequest::new(
            acct,
            "contact",
            daemon,
            serde_json::json!({ "request_id": request_id }),
        ),
        refresh,
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
    use serde_json::json;

    use super::*;

    #[test]
    fn clearing_old_entries_sends_the_setting_with_the_clear_action() {
        let plan = request_call(
            Some("a"),
            methods::CONTACT_REQUESTS_POLICY,
            &json!({ "contact_requests": "manual", "clear_retired_authority_floors": true }),
        );
        let Plan::Call { request, .. } = plan else { panic!("a call") };
        assert_eq!(request.method, "contact.requests.policy.set");
        assert_eq!(
            request.body,
            json!({ "contact_requests": "manual", "clear_retired_authority_floors": true })
        );
        // The plain setting never carries it.
        let plain = request_call(
            Some("a"),
            methods::CONTACT_REQUESTS_POLICY,
            &json!({ "contact_requests": "manual" }),
        );
        let Plan::Call { request, .. } = plain else { panic!("a call") };
        assert_eq!(request.body, json!({ "contact_requests": "manual" }));
    }
}
