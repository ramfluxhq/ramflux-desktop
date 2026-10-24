// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Sends: direct messages, group messages and attachments.
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
//!
//! An attachment travels with a direct message (`message.submit`
//! `attachments`), read from the chosen file at send time and limited to
//! what one frame carries. It goes through the relay of the account's node,
//! named as the daemon names it. Groups have no attachments on the bus.

use ramflux_desktop_core::{ContextId, ContextKind, methods};
use ramflux_sdk::{
    LocalBusFederationRoute, LocalBusGroupSendRequest, LocalBusMessageAttachmentInput,
    LocalBusMessageSubmitRequest, MAX_LOCAL_BUS_ONE_SHOT_OBJECT_BYTES,
};
use serde_json::{Value, json};

use super::fetch::{Fetch, Paged};
use super::{Known, Plan, refuse, refuse_invalid, text};
use crate::bus::BusRequest;
use crate::capabilities;
use crate::translate::reason::{key, keyed};

/// Time to live of a direct message, in seconds (the reference CLI's default).
pub const MESSAGE_TTL: u32 = 300;

/// Relay chunk size of an attachment.
pub const ATTACHMENT_CHUNK: usize = 4096;

/// The largest attachment one message carries, in bytes: what one
/// local-bus frame holds (the daemon refuses more).
pub const ATTACHMENT_LIMIT: usize = MAX_LOCAL_BUS_ONE_SHOT_OBJECT_BYTES;

/// A fresh id with `prefix`.
pub(super) fn fresh_id(prefix: &str) -> Result<String, String> {
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
        Some(ContextKind::Direct) => direct(acct, &context, text(body, "text"), Vec::new(), known),
        Some(kind) => refuse(
            capabilities::reason_for(&methods::in_context(methods::SEND_MESSAGE, kind))
                .unwrap_or(key::NO_ROUTE),
        ),
        None => refuse_invalid("unknown conversation"),
    }
}

/// Plans `object.send`: a direct message carrying the file.
pub(super) fn attach(acct: Option<&str>, body: &Value, known: &impl Known) -> Plan {
    let context = ContextId::new(text(body, "context_id"));
    match known.context_kind(&context) {
        Some(ContextKind::Direct) => {}
        Some(kind) => {
            return refuse(
                capabilities::reason_for(&methods::in_context(methods::ATTACH_OBJECT, kind))
                    .unwrap_or(key::NO_ROUTE),
            );
        }
        None => return refuse_invalid("unknown conversation"),
    }
    let Some(relay) = known.directory().and_then(|d| d.relay_endpoint.clone()) else {
        return refuse(key::NO_RELAY);
    };
    let path = std::path::Path::new(text(body, "path"));
    let limit = ATTACHMENT_LIMIT;
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > limit as u64 => {
            return refuse(&keyed(key::ATTACHMENT_TOO_LARGE, &[("limit", &limit.to_string())]));
        }
        Ok(_) => {}
        Err(error) => return refuse(&keyed(key::FILE, &[("reason", &error.to_string())])),
    }
    let bytes = match std::fs::read(path) {
        Ok(bytes) => zeroize::Zeroizing::new(bytes),
        Err(error) => return refuse(&keyed(key::FILE, &[("reason", &error.to_string())])),
    };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let attachment = Attachment { bytes: &bytes, name, relay };
    direct(acct, &context, "", vec![attachment], known)
}

/// A file to attach.
struct Attachment<'a> {
    bytes: &'a [u8],
    name: Option<String>,
    relay: String,
}

fn direct(
    acct: Option<&str>,
    context: &ContextId,
    message: &str,
    attachments: Vec<Attachment<'_>>,
    known: &impl Known,
) -> Plan {
    let Some(directory) = known.directory() else { return refuse(key::NO_ROUTE) };
    let Some(route) = directory.conversations.get(context).and_then(|c| c.direct_route.as_ref())
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
    let has_attachments = !attachments.is_empty();
    let inputs: Vec<(LocalBusMessageAttachmentInput, Option<String>)> = attachments
        .into_iter()
        .enumerate()
        .map(|(index, a)| {
            let input = LocalBusMessageAttachmentInput {
                name: None,
                mime: None,
                object_id: format!("attachment:{message_id}:{index}"),
                plaintext_base64: ramflux_protocol::encode_base64url(a.bytes),
                chunk_size: ATTACHMENT_CHUNK,
                relay_endpoint: a.relay,
                owner_home_node_id: None,
                relay_audience_node_id: None,
                relay_service_key_base64: None,
            };
            (input, a.name)
        })
        .collect();
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
        attachments: inputs,
    });
    let mut refresh = vec![Fetch::Messages(context.clone()), Fetch::first(Paged::Conversations)];
    if has_attachments {
        refresh.push(Fetch::first(Paged::Objects));
    }
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
    pub(super) attachments: Vec<(LocalBusMessageAttachmentInput, Option<String>)>,
}

/// The `message.submit` body, built from the SDK's request type. A file
/// name rides along on its attachment (the daemon records it as the
/// object's name).
pub(super) fn submit_request(parts: SubmitParts<'_>) -> Result<Value, serde_json::Error> {
    let names: Vec<Option<String>> = parts.attachments.iter().map(|(_, n)| n.clone()).collect();
    let submit = LocalBusMessageSubmitRequest {
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
        attachments: parts.attachments.into_iter().map(|(input, _)| input).collect(),
        federation: parts.federation,
    };
    let mut body = serde_json::to_value(submit)?;
    if let Some(list) = body.get_mut("attachments").and_then(Value::as_array_mut) {
        for (attachment, name) in list.iter_mut().zip(names) {
            if let (Some(name), Value::Object(map)) = (name, attachment) {
                map.insert("name".to_owned(), json!(name));
            }
        }
    }
    Ok(body)
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
    let title = known.kept_title(context).map(str::to_owned);
    match serde_json::to_value(send) {
        Ok(mut body) => {
            if let Some(title) = &title {
                body["title"] = json!(title);
            }
            Plan::Call {
                request: BusRequest::new(acct, "group", "group.send", body),
                refresh: vec![Fetch::GroupReceive {
                    context: context.clone(),
                    group: group.to_owned(),
                    title,
                }],
            }
        }
        Err(error) => refuse_invalid(&error.to_string()),
    }
}
