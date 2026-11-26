// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Sends: direct and group text messages (attachments: see `media`).
//!
//! A direct message is `message.submit` over the conversation's route as
//! `conversation.directory` lists it: the route's first (selected) device,
//! its delivery target, the recipient's commitment and, for a peer on
//! another node, the federation route. This account's principal and device
//! are the source and sender. A conversation without an active route is
//! refused with that reason; no route is ever looked up or guessed.
//!
//! A group message is `group.send` to the group the conversation names,
//! without delivery fields, so the daemon resolves every member's route
//! before anything is encrypted or stored, and refuses the whole send when
//! one is missing (see `translate::send_failures`). It carries the name this
//! account keeps for the conversation: the daemon records the name from
//! every group call that names the conversation.

use ramflux_desktop_core::{ContextId, ContextKind, methods};
use ramflux_sdk::{
    LocalBusFederationRoute, LocalBusGroupSendRequest, LocalBusMessageSubmitRequest,
};
use serde_json::Value;

use super::fetch::{Fetch, Paged};
use super::{Known, Plan, refuse, refuse_invalid, text};
use crate::bus::BusRequest;
use crate::capabilities;
use crate::translate::reason::key;

/// Time to live of a direct message, in seconds (the reference CLI's default).
pub const MESSAGE_TTL: u32 = 300;

/// A fresh id with `prefix`.
pub(crate) fn fresh_id(prefix: &str) -> Result<String, String> {
    let random = zeroize::Zeroizing::new(
        ramflux_crypto::random_32().map_err(|error| format!("no randomness: {error}"))?,
    );
    let hex: String = random.iter().take(16).map(|b| format!("{b:02x}")).collect();
    Ok(format!("{prefix}{hex}"))
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn encoded(text: &str) -> String {
    ramflux_protocol::encode_base64url(text.as_bytes())
}

/// Plans `message.send`.
pub(super) fn send(acct: Option<&str>, body: &Value, known: &impl Known) -> Plan {
    let context = ContextId::new(text(body, "context_id"));
    match known.context_kind(&context) {
        Some(ContextKind::Group) => group_send(acct, &context, text(body, "text"), known),
        Some(ContextKind::Direct) => direct(acct, &context, text(body, "text"), known),
        Some(kind) => refuse(
            capabilities::reason_for(&methods::in_context(methods::SEND_MESSAGE, kind))
                .unwrap_or(key::NO_ROUTE),
        ),
        None => refuse_invalid("unknown conversation"),
    }
}

/// The route to the other participant of a direct conversation whose
/// route the daemon has not captured (one this account did not start: the
/// daemon captures routes from its own submissions). As the read model
/// says, the client selects an active device: the peer's first verified
/// device with a signing key and a delivery target, on this home node.
fn route_from_identity(
    directory: &crate::directory::Directory,
    participants: &[String],
) -> Option<crate::directory::DirectRoute> {
    let me = directory.self_principal.as_deref();
    let mut peers = participants.iter().filter(|p| Some(p.as_str()) != me);
    let (peer, None) = (peers.next()?, peers.next()) else { return None };
    let identity = directory.identities.get(peer)?;
    let device = identity.devices.iter().find(|d| d.routable())?;
    Some(crate::directory::DirectRoute {
        commitment: Some(identity.commitment.clone()?),
        device_ids: vec![device.device_id.clone()],
        target: device.target.clone(),
        federation: None,
    })
}

fn direct(acct: Option<&str>, context: &ContextId, message: &str, known: &impl Known) -> Plan {
    let Some(directory) = known.directory() else { return refuse(key::NO_ROUTE) };
    let Some(conversation) = directory.conversations.get(context) else {
        return refuse(key::NO_ROUTE);
    };
    let Some(route) = conversation
        .direct_route
        .clone()
        .or_else(|| route_from_identity(directory, &conversation.participants))
    else {
        return refuse(key::NO_ROUTE);
    };
    let (Some(device), Some(target)) = (route.device_ids.first(), route.target.as_ref()) else {
        return refuse(key::NO_ROUTE);
    };
    let (Some(source), Some(sender)) = (directory.self_principal.as_ref(), known.self_device())
    else {
        return refuse(key::NO_SELF);
    };
    let federation = match route.federation.clone().map(serde_json::from_value) {
        None => None,
        Some(Ok(route)) => Some(route),
        Some(Err(_)) => return refuse(key::NO_ROUTE),
    };
    // One id for the message and its envelope: the daemon binds the
    // envelope id into the AEAD associated data, and the recipient's daemon
    // (and the receipt) name the message by it.
    let (message_id, envelope_id) = match fresh_id("desk-msg-") {
        Ok(id) => (id.clone(), id),
        Err(error) => return refuse_invalid(&error),
    };
    let request = submit_request(SubmitParts {
        conversation: context,
        message_id,
        envelope_id,
        source,
        sender,
        device,
        commitment: route.commitment.clone(),
        target,
        text: message,
        federation,
    });
    let refresh = vec![Fetch::Messages(context.clone()), Fetch::first(Paged::Conversations)];
    match request {
        Ok(body) => Plan::Call {
            request: BusRequest::new(acct, "message", "message.submit", body),
            refresh,
        },
        Err(error) => refuse_invalid(&error.to_string()),
    }
}

pub(super) struct SubmitParts<'a> {
    pub(super) conversation: &'a ContextId,
    pub(super) message_id: String,
    pub(super) envelope_id: String,
    pub(super) source: &'a str,
    pub(super) sender: &'a str,
    pub(super) device: &'a str,
    pub(super) commitment: Option<String>,
    pub(super) target: &'a str,
    pub(super) text: &'a str,
    pub(super) federation: Option<LocalBusFederationRoute>,
}

/// The `message.submit` body, built from the SDK's request type.
pub(super) fn submit_request(parts: SubmitParts<'_>) -> Result<Value, serde_json::Error> {
    let submit = LocalBusMessageSubmitRequest {
        content: None,
        conversation_id: parts.conversation.to_string(),
        message_id: parts.message_id,
        envelope_id: parts.envelope_id,
        source_principal_id: parts.source.to_owned(),
        sender_id: parts.sender.to_owned(),
        recipient_device_id: Some(parts.device.to_owned()),
        recipient_principal_commitment: parts.commitment,
        target_delivery_id: parts.target.to_owned(),
        encrypted_body_base64: String::new(),
        plaintext_body_base64: Some(encoded(parts.text)),
        created_at: now_seconds(),
        ttl: MESSAGE_TTL,
        attachments: Vec::new(),
        federation: parts.federation,
    };
    serde_json::to_value(submit)
}

fn group_send(acct: Option<&str>, context: &ContextId, message: &str, known: &impl Known) -> Plan {
    let Some(group) = known.directory().and_then(|d| d.group_of(context)) else {
        return refuse(key::NO_GROUP_LINK);
    };
    let Some(sender) = known.self_device() else { return refuse(key::NO_SELF) };
    let message_id = match fresh_id("desk-msg-") {
        Ok(id) => id,
        Err(error) => return refuse_invalid(&error),
    };
    let send = LocalBusGroupSendRequest {
        content: None,
        group_id: group.to_owned(),
        conversation_id: context.to_string(),
        message_id,
        sender_id: sender.to_owned(),
        encrypted_body_base64: String::new(),
        plaintext_body_base64: Some(encoded(message)),
        envelope_id: None,
        source_principal_id: None,
        target_delivery_id: None,
        federation: None,
        ttl: None,
    };
    match serde_json::to_value(send) {
        Ok(body) => Plan::Call {
            request: BusRequest::new(acct, "group", "group.send", body),
            refresh: vec![Fetch::Messages(context.clone())],
        },
        Err(error) => refuse_invalid(&error.to_string()),
    }
}

/// What an action on a listed message names: the daemon's id of the
/// message (base64url of its event id) and the edit revision shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageTarget {
    /// The daemon's message id.
    pub daemon_id: String,
    /// The highest applied edit revision (0 when unedited).
    pub revision: u64,
}

/// Plans a reply, an edit or a delete for everyone: the daemon's
/// conversation mutations, which build, sign and deliver the event to every
/// recipient of the conversation (direct or group) themselves.
///
/// - `conversation.reply` `{conversation_id, message_id, text}`;
/// - `conversation.edit` `{conversation_id, message_id, text,
///   expected_revision}`: the revision shown, so an edit made meanwhile on
///   another device is refused (`edit_conflict`) instead of overwritten;
/// - `conversation.delete` `{conversation_id, message_id, scope}`, the scope
///   the intent named: `local` (this device) or `everyone`.
///
/// The frame's retry key makes a resend the same operation.
pub(super) fn mutate(acct: Option<&str>, method: &str, body: &Value, known: &impl Known) -> Plan {
    let context = ContextId::new(text(body, "context_id"));
    let moment = ramflux_desktop_core::MomentId::new(text(body, "moment_id"));
    if known.context_kind(&context).is_none() {
        return refuse_invalid("unknown conversation");
    }
    let Some(target) = known.message_target(&moment) else {
        return refuse_invalid("the message is not listed by the daemon yet");
    };
    let (daemon, request) = match method {
        methods::MESSAGE_REPLY => (
            "conversation.reply",
            serde_json::json!({
                "conversation_id": context,
                "message_id": target.daemon_id,
                "text": text(body, "text"),
            }),
        ),
        methods::MESSAGE_EDIT => (
            "conversation.edit",
            serde_json::json!({
                "conversation_id": context,
                "message_id": target.daemon_id,
                "text": text(body, "text"),
                "expected_revision": target.revision,
            }),
        ),
        _ => {
            let scope = text(body, "scope");
            if !matches!(scope, "local" | "everyone") {
                return refuse_invalid("unknown delete scope");
            }
            (
                "conversation.delete",
                serde_json::json!({
                    "conversation_id": context,
                    "message_id": target.daemon_id,
                    "scope": scope,
                }),
            )
        }
    };
    Plan::Call {
        request: BusRequest::new(acct, "message", daemon, request),
        refresh: vec![Fetch::Messages(context), Fetch::first(Paged::Conversations)],
    }
}
