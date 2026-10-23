// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Starting a direct conversation: its first message.
//!
//! The daemon has no "create conversation" request: a direct conversation
//! exists once a message was submitted in it, and `conversation.directory`
//! lists it with the route of that submission. So the desktop sends the
//! first message with `message.submit` under a fresh conversation id (a
//! random UUID v4; never derived from either identity) to a route it takes
//! from `identity.directory`:
//!
//! - the contact's principal commitment;
//! - the first of the contact's devices, in the daemon's order, that has
//!   both a verified signing key and a delivery target.
//!
//! When no such device is listed the start is refused, saying which part
//! is missing (the identity, a delivery target, or a verified key), so the
//! user knows what to do; no route is guessed. The contact's name as the
//! user sees it rides along as the conversation's local title (the daemon
//! records `title` from the submission).

use ramflux_desktop_core::ContextId;
use serde_json::{Value, json};

use super::fetch::{Fetch, Paged};
use super::message::{SubmitParts, fresh_id, submit_request};
use super::{Known, Plan, refuse, refuse_invalid, text};
use crate::bus::BusRequest;
use crate::directory::{Identity, IdentityDevice};
use crate::translate::reason::{key, keyed};

/// A fresh conversation id: a random UUID (version 4, RFC 9562 layout).
///
/// # Errors
///
/// Returns the reason when the system has no randomness.
pub fn fresh_conversation_id() -> Result<ContextId, String> {
    let mut bytes = zeroize::Zeroizing::new(
        ramflux_crypto::random_32().map_err(|error| format!("no randomness: {error}"))?,
    );
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().take(16).map(|b| format!("{b:02x}")).collect();
    Ok(ContextId::new(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )))
}

/// Why no route to a contact can be chosen, if none can; else the device.
fn route<'a>(identity: Option<&'a Identity>, name: &str) -> Result<&'a IdentityDevice, String> {
    let named = |reason: &str| keyed(reason, &[("name", name)]);
    let Some(identity) = identity else { return Err(named(key::START_NO_IDENTITY)) };
    if let Some(device) = identity.devices.iter().find(|d| d.routable()) {
        return Ok(device);
    }
    if identity.devices.iter().any(|d| d.target.is_some()) {
        // A target without a verified signing key: the key is not verified.
        Err(named(key::START_UNVERIFIED_KEY))
    } else {
        Err(named(key::START_NO_TARGET))
    }
}

/// Plans `conversation.start`: the first message under the new id.
pub(super) fn start(acct: Option<&str>, body: &Value, known: &impl Known) -> Plan {
    let context = ContextId::new(text(body, "context_id"));
    let participant = text(body, "participant_id");
    if context.as_str().is_empty() || participant.is_empty() {
        return refuse_invalid("no conversation or contact named");
    }
    if known.context_kind(&context).is_some() {
        return refuse_invalid("the conversation already exists");
    }
    let title = body["title"].as_str().filter(|t| !t.trim().is_empty());
    let name = title.unwrap_or(participant);
    let Some(directory) = known.directory() else { return refuse(key::NO_SELF) };
    let identity = directory.identities.get(participant);
    let device = match route(identity, name) {
        Ok(device) => device,
        Err(reason) => return refuse(&reason),
    };
    let (Some(commitment), Some(target)) =
        (identity.and_then(|i| i.commitment.clone()), device.target.as_deref())
    else {
        return refuse(key::NO_COMMITMENT);
    };
    let (Some(source), Some(sender)) = (directory.self_principal.as_ref(), known.self_device())
    else {
        return refuse(key::NO_SELF);
    };
    let ids = fresh_id("desk-msg-").and_then(|m| Ok((m, fresh_id("desk-env-")?)));
    let (message_id, envelope_id) = match ids {
        Ok(ids) => ids,
        Err(error) => return refuse_invalid(&error),
    };
    let request = submit_request(SubmitParts {
        conversation: &context,
        message_id,
        envelope_id,
        source,
        sender,
        device: &device.device_id,
        commitment: Some(commitment),
        target,
        text: text(body, "text"),
        federation: None,
        attachments: Vec::new(),
    });
    match request {
        Ok(mut request) => {
            if let (Some(title), Value::Object(map)) = (title, &mut request) {
                map.insert("title".to_owned(), json!(title));
            }
            Plan::Call {
                request: BusRequest::new(acct, "message", "message.submit", request),
                refresh: vec![Fetch::first(Paged::Conversations), Fetch::Messages(context.clone())],
            }
        }
        Err(error) => refuse_invalid(&error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_ids_are_version_4_uuids_and_differ() {
        let a = fresh_conversation_id().expect("id");
        let b = fresh_conversation_id().expect("id");
        assert_ne!(a, b);
        let parts: Vec<&str> = a.as_str().split('-').collect();
        assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), [8, 4, 4, 4, 12]);
        assert!(parts[2].starts_with('4'));
        assert!(matches!(parts[3].as_bytes()[0], b'8' | b'9' | b'a' | b'b'));
    }

    #[test]
    fn a_route_needs_a_device_with_key_and_target() {
        let device = |key: Option<&str>, target: Option<&str>| IdentityDevice {
            device_id: "d".to_owned(),
            signing_key: key.map(str::to_owned),
            target: target.map(str::to_owned),
        };
        let identity = |devices| Identity { commitment: Some("c".into()), is_self: false, devices };
        let reason = |r: Result<&IdentityDevice, String>| r.err().unwrap_or_default();
        assert!(reason(route(None, "Bob")).starts_with(key::START_NO_IDENTITY));
        let none = identity(vec![device(Some("k"), None)]);
        assert!(reason(route(Some(&none), "Bob")).starts_with(key::START_NO_TARGET));
        let unverified = identity(vec![device(None, Some("t"))]);
        assert!(reason(route(Some(&unverified), "Bob")).starts_with(key::START_UNVERIFIED_KEY));
        let ok = identity(vec![device(None, Some("t")), device(Some("k"), Some("t2"))]);
        assert_eq!(route(Some(&ok), "Bob").map(|d| d.target.clone()), Ok(Some("t2".into())));
    }
}
