// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Typed message schema v1 updates to domain events (S10c).
//!
//! Input is the SDK's generated `MessageUpdate`; nothing here reads a hand
//! written key, byte prefix or recovered text. The projection is dormant:
//! production still reads the legacy rows, and only the test seam
//! [`candidate`] (feature `test-support`) reaches it until the live decode
//! path lands.
//!
//! - `message`: a message and its scalar-offset spans, reply and forward.
//! - `service`: group service events (membership, rename, key change).

#[cfg(feature = "test-support")]
pub mod candidate;
mod message;
mod service;

use ramflux_desktop_core::{
    ContextId, DomainEvent, FallbackPayload, Lifecycle, Moment, MomentId, MomentKind, OrderKey,
    Timestamp, UnsupportedInfo,
};
use ramflux_protocol::message::{
    EventIdentity, MessageUpdate, QualifiedDevice, content::Value as ContentValue,
    message_update::State,
};

use super::Translator;
use super::conversation::message_moment_id;

/// Projects one update; updates this part does not own yet project to nothing.
pub fn project(translator: &mut Translator, update: &MessageUpdate) -> Vec<DomainEvent> {
    let (Some(identity), Some(State::Content(content))) = (&update.identity, &update.state) else {
        return Vec::new();
    };
    match &content.value {
        Some(ContentValue::Message(message)) => {
            message::project(translator, identity, message).into_iter().collect()
        }
        Some(ContentValue::Service(service)) => service::project(translator, identity, service),
        _ => Vec::new(),
    }
}

/// The context an event identity belongs to.
fn context_of(identity: &EventIdentity) -> ContextId {
    ContextId::new(&identity.conversation_id)
}

/// Lowercase hex of an event or message id.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
        out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
        out
    })
}

/// The moment id of an event in `context`.
fn moment_id(context: &ContextId, id: &[u8]) -> MomentId {
    message_moment_id(context, &hex(id))
}

/// The author device as the daemon names it, for the order key.
fn author_key(author: Option<&QualifiedDevice>) -> String {
    author.map_or_else(String::new, |a| format!("{}/{}", a.account_anchor, a.device_id))
}

/// The order key of an event (D4): clock, author, device sequence.
fn order_of(identity: &EventIdentity) -> OrderKey {
    OrderKey {
        hlc: identity.hlc,
        author: author_key(identity.author.as_ref()),
        device_sequence: identity.device_sequence,
    }
}

/// A moment of `identity` with its stream position; display time is not
/// carried by the typed update, so it stays zero until the live path sets it.
fn moment_of(identity: &EventIdentity, id: MomentId, origin: &str, kind: MomentKind) -> Moment {
    Moment {
        id,
        context_id: context_of(identity),
        origin: ramflux_desktop_core::ParticipantId::new(origin),
        at: Timestamp(0),
        kind,
        lifecycle: Lifecycle::Done,
        order: Some(order_of(identity)),
    }
}

/// The placeholder for a whole update this build cannot show: no partial
/// text, no link, no action.
fn unsupported(reason: &str, machine: &str) -> MomentKind {
    MomentKind::Fallback(FallbackPayload {
        reason: format!("backend.message.{reason}"),
        text: String::new(),
        unsupported: Some(UnsupportedInfo {
            kind: "message".to_owned(),
            layer: "1".to_owned(),
            reason: machine.to_owned(),
            raw_length: 0,
        }),
    })
}
