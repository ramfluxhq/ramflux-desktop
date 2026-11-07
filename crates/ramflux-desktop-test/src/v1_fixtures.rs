// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Message schema v1 updates for tests, built only with the generated types.

use ramflux_protocol::message::{
    Content, CreateSurfaceCommand, Delete, DeleteScope, Edit, Entity, EntityKind, EventIdentity,
    Forward, JsonArray, JsonMember, JsonNull, JsonObject, JsonValue, MediaRef, Message, MessageRef,
    MessageUpdate, QualifiedDevice, Reaction, Receipt, ReceiptState, Service, ServiceAction,
    Surface, SurfaceAction, SurfaceCarrier, SurfaceCommand, SurfaceCommandCreateSurface,
    SurfaceDelivery, SurfaceEvent, SurfaceSnapshotProof, Thumbnail, UnsupportedView,
    content::Value, json_value, message_update::State, surface::Payload as SurfacePayload,
    surface_carrier::Payload as CarrierPayload, surface_command::Value as Command,
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

/// A real 1x1 PNG, JPEG and WebP: bytes the image decoders accept.
pub const REAL_PNG: &[u8] = include_bytes!("../assets/1x1.png");
/// See [`REAL_PNG`].
pub const REAL_JPEG: &[u8] = include_bytes!("../assets/1x1.jpg");
/// See [`REAL_PNG`].
pub const REAL_WEBP: &[u8] = include_bytes!("../assets/1x1.webp");

/// A 1x1 thumbnail of `mime` made of real image bytes.
#[must_use]
pub fn real_thumbnail(mime: &str) -> Thumbnail {
    let bytes = match mime {
        "image/jpeg" => REAL_JPEG,
        "image/webp" => REAL_WEBP,
        _ => REAL_PNG,
    };
    Thumbnail { mime_type: mime.to_owned(), width: 1, height: 1, encoded_image: bytes.to_vec() }
}

/// An edit of `target` to `revision` with the replacement `message`.
#[must_use]
pub fn edit_update(
    identity: EventIdentity,
    target: MessageRef,
    revision: u64,
    message: Message,
) -> MessageUpdate {
    content_update(
        identity,
        Value::Edit(Edit {
            target: Some(target),
            revision,
            previous_content_digest: Vec::new(),
            replacement: Some(message),
        }),
    )
}

/// A delete of `targets` with `scope`.
#[must_use]
pub fn delete_update(
    identity: EventIdentity,
    targets: Vec<MessageRef>,
    scope: DeleteScope,
) -> MessageUpdate {
    content_update(
        identity,
        Value::Delete(Delete {
            targets,
            scope: scope as i32,
            tombstone_id: Vec::new(),
            native_group_authorization: Vec::new(),
        }),
    )
}

/// A reaction of `token` on `target` (`remove` takes it back).
#[must_use]
pub fn reaction_update(
    identity: EventIdentity,
    target: MessageRef,
    token: &str,
    remove: bool,
) -> MessageUpdate {
    content_update(
        identity,
        Value::Reaction(Reaction {
            target: Some(target),
            token: token.to_owned(),
            remove,
            operation_id: Vec::new(),
        }),
    )
}

/// A receipt of `state` for `targets` from the device `receiver`.
#[must_use]
pub fn receipt_update(
    identity: EventIdentity,
    targets: Vec<MessageRef>,
    state: ReceiptState,
    receiver: &str,
) -> MessageUpdate {
    content_update(
        identity,
        Value::Receipt(Receipt {
            targets,
            state: state as i32,
            at: 0,
            receiver: Some(device(receiver)),
        }),
    )
}

/// `update` with the journal sequence `journal` (a reparse of the same
/// event is journaled again).
#[must_use]
pub fn with_journal(mut update: MessageUpdate, journal: u64) -> MessageUpdate {
    update.journal_sequence = journal;
    update
}

/// An update the SDK could not parse: layer, kind and stored length, plus the
/// raw digest and the handle that no desktop model may keep.
#[must_use]
pub fn unsupported_update(
    identity: EventIdentity,
    journal: u64,
    layer: u32,
    kind: u32,
    raw_length: u32,
) -> MessageUpdate {
    MessageUpdate {
        account_id: ACCOUNT.to_owned(),
        journal_sequence: journal,
        identity: Some(identity),
        state: Some(State::Unsupported(UnsupportedView {
            layer,
            kind,
            raw_digest: b"RAW-DIGEST-BYTES".to_vec(),
            raw_length,
            raw_handle: b"RAW-HANDLE-SECRET".to_vec(),
            reason: "unknown_kind".to_owned(),
        })),
    }
}

/// A typed JSON tree of `value`.
#[must_use]
pub fn json_value(value: &serde_json::Value) -> JsonValue {
    use serde_json::Value as Json;
    JsonValue {
        value: Some(match value {
            Json::Null => json_value::Value::Null(JsonNull {}),
            Json::Bool(flag) => json_value::Value::Boolean(*flag),
            Json::Number(number) => json_value::Value::Number(number.to_string()),
            Json::String(text) => json_value::Value::Text(text.clone()),
            Json::Array(items) => json_value::Value::Array(JsonArray {
                values: items.iter().map(json_value).collect(),
            }),
            Json::Object(map) => json_value::Value::Object(JsonObject {
                members: map
                    .iter()
                    .map(|(key, inner)| JsonMember {
                        key: key.clone(),
                        value: Some(json_value(inner)),
                    })
                    .collect(),
            }),
        }),
    }
}

/// One created surface and who is involved, built into carriers.
#[derive(Debug, Clone)]
pub struct SurfaceSpec {
    /// Conversation.
    pub conversation: String,
    /// The surface's own id.
    pub surface_id: String,
    /// Its author principal.
    pub author: String,
    /// Catalog name.
    pub catalog: String,
    /// Catalog version.
    pub catalog_version: String,
    /// Text for clients that cannot draw it.
    pub fallback_text: String,
    /// Components, as plain JSON.
    pub components: Vec<serde_json::Value>,
    /// Allowed actions, as plain JSON.
    pub actions: Vec<serde_json::Value>,
    /// The version the event creates.
    pub version: u64,
    /// The snapshot hash the carrier attests.
    pub hash: String,
}

impl SurfaceSpec {
    /// A `basic/1` surface with one text block and one action.
    #[must_use]
    pub fn basic(conversation: &str, surface_id: &str, author: &str) -> Self {
        Self {
            conversation: conversation.to_owned(),
            surface_id: surface_id.to_owned(),
            author: author.to_owned(),
            catalog: "basic".to_owned(),
            catalog_version: "1".to_owned(),
            fallback_text: "Report is ready".to_owned(),
            components: vec![
                serde_json::json!({ "id": "b-text", "type": "Text", "text": "Report" }),
            ],
            actions: vec![serde_json::json!({ "name": "open", "label": "Open report" })],
            version: 1,
            hash: format!("hash-{surface_id}-1"),
        }
    }

    /// The same surface in another catalog.
    #[must_use]
    pub fn in_catalog(mut self, catalog: &str, version: &str) -> Self {
        self.catalog = catalog.to_owned();
        self.catalog_version = version.to_owned();
        self
    }

    /// The same surface at `version`.
    #[must_use]
    pub fn at_version(mut self, version: u64) -> Self {
        self.version = version;
        self.hash = format!("hash-{}-{version}", self.surface_id);
        self
    }

    fn carrier(&self, message_id: &str, sender: &str) -> SurfaceCarrier {
        SurfaceCarrier {
            schema: "ramflux.surface.carrier.v2".to_owned(),
            content_type: "surface".to_owned(),
            posted_at: Some(1_000),
            updated_at: None,
            conversation_id: self.conversation.clone(),
            surface_id: self.surface_id.clone(),
            message_id: message_id.to_owned(),
            sender_principal_id: sender.to_owned(),
            sender_device_id: format!("dev-{sender}"),
            author_principal_id: self.author.clone(),
            signature: String::new(),
            snapshot: None,
            payload: None,
        }
    }

    /// The carrier of the create event.
    #[must_use]
    pub fn create_carrier(&self, message_id: &str) -> SurfaceCarrier {
        let mut carrier = self.carrier(message_id, &self.author);
        carrier.snapshot = Some(SurfaceSnapshotProof {
            surface_hash: Some(self.hash.clone()),
            ..SurfaceSnapshotProof::default()
        });
        carrier.payload = Some(CarrierPayload::Event(SurfaceEvent {
            schema: "ramflux.a2ui.conversation.v2".to_owned(),
            conversation_id: self.conversation.clone(),
            version: self.version,
            command: Some(SurfaceCommand {
                value: Some(Command::CreateSurface(SurfaceCommandCreateSurface {
                    value: Some(CreateSurfaceCommand {
                        surface_id: self.surface_id.clone(),
                        surface_type: None,
                        catalog: self.catalog.clone(),
                        catalog_version: self.catalog_version.clone(),
                        fallback_text: self.fallback_text.clone(),
                        fallback_markdown: None,
                        components: self.components.iter().map(json_value).collect(),
                        data_model: None,
                        allowed_actions: self.actions.iter().map(json_value).collect(),
                    }),
                })),
            }),
        }));
        carrier
    }

    /// The carrier of `requester` asking for `action_id` on version `version`.
    #[must_use]
    pub fn action_carrier(
        &self,
        message_id: &str,
        requester: &str,
        action_id: &str,
        version: u64,
    ) -> SurfaceCarrier {
        let mut carrier = self.carrier(message_id, requester);
        carrier.payload = Some(CarrierPayload::Action(SurfaceAction {
            schema: "ramflux.a2ui.action.v2".to_owned(),
            conversation_id: self.conversation.clone(),
            surface_id: self.surface_id.clone(),
            version,
            surface_hash: self.hash.clone(),
            action_id: action_id.to_owned(),
            values: None,
            args_hash: String::new(),
            task_binding: None,
        }));
        carrier
    }

    /// The author's receipt holding `original`.
    #[must_use]
    pub fn receipt_carrier(&self, message_id: &str, original: SurfaceCarrier) -> SurfaceCarrier {
        let mut carrier = self.carrier(message_id, &self.author);
        carrier.payload = Some(CarrierPayload::ActionReceipt(Box::new(original)));
        carrier
    }
}

/// A content update carrying a surface delivery of `carrier`.
#[must_use]
pub fn surface_update(identity: EventIdentity, carrier: SurfaceCarrier) -> MessageUpdate {
    content_update(
        identity,
        Value::Surface(Surface {
            payload: Some(SurfacePayload::Delivery(SurfaceDelivery {
                kind: None,
                carrier: Some(carrier),
                group_epoch: None,
                context: None,
                manifests: Vec::new(),
            })),
        }),
    )
}
