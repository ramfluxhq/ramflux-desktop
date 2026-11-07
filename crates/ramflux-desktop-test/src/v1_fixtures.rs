// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Message schema v1 updates for tests, built only with the generated types.

use ramflux_protocol::message::{
    Content, Entity, EntityKind, EventIdentity, Forward, MediaRef, Message, MessageRef,
    MessageUpdate, QualifiedDevice, Service, ServiceAction, Thumbnail, content::Value,
    message_update::State,
};

/// The account the fixtures belong to.
pub const ACCOUNT: &str = "acct-main";

/// A qualified device of `device_id`.
#[must_use]
pub fn device(device_id: &str) -> QualifiedDevice {
    QualifiedDevice {
        account_anchor: format!("anchor-{device_id}"),
        device_id: device_id.to_owned(),
        device_epoch: 1,
    }
}

/// An event identity in `conversation` from `author`.
#[must_use]
pub fn identity(
    conversation: &str,
    id: &[u8],
    author: &str,
    hlc: u64,
    device_sequence: u64,
) -> EventIdentity {
    EventIdentity {
        id: id.to_vec(),
        conversation_id: conversation.to_owned(),
        author: Some(device(author)),
        device_sequence,
        hlc,
        group_id: None,
        group_epoch: None,
    }
}

/// An entity of `kind` over `offset..offset + length` scalars.
#[must_use]
pub fn entity(offset: u32, length: u32, kind: EntityKind) -> Entity {
    Entity { offset, length, kind: kind as i32, value: None }
}

/// An entity of `kind` carrying `value` (a link target, a mention).
#[must_use]
pub fn entity_with_value(
    offset: u32,
    length: u32,
    kind: EntityKind,
    value: Option<&str>,
) -> Entity {
    Entity { offset, length, kind: kind as i32, value: value.map(str::to_owned) }
}

/// An entity whose kind number is not a known `EntityKind`.
#[must_use]
pub fn unknown_entity(offset: u32, length: u32, kind: i32) -> Entity {
    Entity { offset, length, kind, value: None }
}

/// A message of `text` with `entities`.
#[must_use]
pub fn text_message(text: &str, entities: Vec<Entity>) -> Message {
    Message {
        text: text.to_owned(),
        entities,
        reply_to: None,
        forward: None,
        media: Vec::new(),
        sealed_group: None,
    }
}

/// A reference to message `id` in `conversation`.
#[must_use]
pub fn message_ref(conversation: &str, id: &[u8]) -> MessageRef {
    MessageRef {
        id: id.to_vec(),
        conversation_id: conversation.to_owned(),
        author_anchor: String::new(),
    }
}

/// A forward of `original`, attributed to `name`.
#[must_use]
pub fn forward_of(original: MessageRef, name: &str) -> Forward {
    Forward {
        original: Some(original),
        original_display_name: Some(name.to_owned()),
        origin_assertion: Vec::new(),
    }
}

/// A content update carrying a message.
#[must_use]
pub fn message_update(identity: EventIdentity, message: Message) -> MessageUpdate {
    content_update(identity, Value::Message(message))
}

/// A content update carrying a service event.
#[must_use]
pub fn service_update(
    identity: EventIdentity,
    action: ServiceAction,
    subjects: &[&str],
    text: Option<&str>,
) -> MessageUpdate {
    content_update(
        identity,
        Value::Service(Service {
            action: action as i32,
            subjects: subjects.iter().map(|d| device(d)).collect(),
            text: text.map(str::to_owned),
            target: None,
            native_authority_proof: Vec::new(),
        }),
    )
}

/// A content update carrying a service event whose action number is unknown.
#[must_use]
pub fn unknown_service_update(identity: EventIdentity, action: i32) -> MessageUpdate {
    content_update(
        identity,
        Value::Service(Service {
            action,
            subjects: Vec::new(),
            text: None,
            target: None,
            native_authority_proof: Vec::new(),
        }),
    )
}

fn content_update(identity: EventIdentity, value: Value) -> MessageUpdate {
    MessageUpdate {
        account_id: ACCOUNT.to_owned(),
        journal_sequence: identity.device_sequence,
        identity: Some(identity),
        state: Some(State::Content(Content { value: Some(value) })),
    }
}

/// A thumbnail of `mime`, `width` by `height`, whose bytes start with the
/// codec's signature and are `len` bytes long.
#[must_use]
pub fn thumbnail(mime: &str, width: u32, height: u32, len: usize) -> Thumbnail {
    let signature: &[u8] = match mime {
        "image/png" => &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
        "image/jpeg" => &[0xff, 0xd8, 0xff],
        "image/webp" => b"RIFF\0\0\0\0WEBP",
        _ => &[],
    };
    let mut bytes = signature.to_vec();
    bytes.resize(len.max(signature.len()), 0);
    bytes.truncate(len.max(signature.len()));
    Thumbnail { mime_type: mime.to_owned(), width, height, encoded_image: bytes }
}

/// A media reference to `object_id` with an optional inline thumbnail.
#[must_use]
pub fn media_ref(
    object_id: &str,
    name: &str,
    mime: &str,
    size: u64,
    thumbnail: Option<Thumbnail>,
) -> MediaRef {
    MediaRef {
        object_id: object_id.to_owned(),
        manifest_digest: Vec::new(),
        mime_type: mime.to_owned(),
        size,
        key_slots: Vec::new(),
        file_name: Some(name.to_owned()),
        inline_thumbnail: thumbnail,
        large_manifest: None,
    }
}
