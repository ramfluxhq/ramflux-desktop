// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Stored messages of conversations and groups, as message moments.
//!
//! The source reads a conversation through the daemon's message read model
//! (`message.read` with `read_model: true`, schema
//! `ramflux.message.read_model.v1`): a newest-first page of rendered rows
//! rebuilt from the durable account store, direct and group conversations
//! alike. Each row names its original event (`id`, base64url, the same id
//! `message.submit` answers with), its author device, whether this account
//! wrote it (`is_own`), its state (`content`, `unsupported` or `tombstone`),
//! its kind, the text in force after edits, and the per-device receipts.
//!
//! Only `message` rows become message moments here. Calls, tasks and
//! surfaces have their own reads (`call.list`, `task.list`, `surface.list`)
//! that come with every message read, and control kinds (edits, reactions,
//! receipts, typing) are already folded into the rows they change.

use ramflux_desktop_core::{
    ContextId, ContextKind, DomainEvent, FallbackPayload, Lifecycle, MessagePayload, Moment,
    MomentId, MomentKind, ParticipantId, Receipt, Timestamp, UnsupportedInfo,
};
use serde_json::Value;

use super::conversation::message_moment_id;
use super::{Translated, Translator, list, text};

/// The contract every read-model page carries.
pub(crate) const READ_MODEL_SCHEMA: &str = "ramflux.message.read_model.v1";

impl Translator {
    /// Remembers the text of a message this device sent, shown until the
    /// read model lists it.
    pub(super) fn remember_sent(&mut self, message_id: &str, body: &str) {
        self.plaintext.insert(&MomentId::new(message_id), body.to_owned());
    }

    /// One read-model page of a known context. A page of another schema is
    /// not read.
    pub(super) fn messages(
        &mut self,
        context: &ContextId,
        value: &Value,
        now: Timestamp,
    ) -> Translated {
        let Some(kind) = self.contexts.get(context).map(|c| c.kind) else {
            return Translated::default();
        };
        if text(value, "schema") != READ_MODEL_SCHEMA {
            return Translated::default();
        }
        let mut events = Vec::new();
        // Oldest first, so moments are added in the order they were written.
        for row in list(value, "messages").iter().rev() {
            let state = text(row, "state");
            if text(row, "id").is_empty() {
                continue;
            }
            if state == "content" && text(row, "kind") != "message" {
                continue;
            }
            if kind == ContextKind::Direct && state == "content" {
                self.note_received(context, row);
            }
            let moment = self.message_moment(context, row, now);
            events.extend(self.report_moment(moment));
        }
        Translated { events, follow: Vec::new() }
    }

    /// Notes a listed direct message from a peer, for its delivery receipt:
    /// never this account's own (`is_own`), nor a deleted one. The receipt
    /// goes to the author device when the identity directory lists it with
    /// a delivery target. Skipped while no target is listed.
    fn note_received(&mut self, context: &ContextId, row: &Value) {
        let message = text(row, "id");
        let sender = author_device(row);
        let Some(me) = self.self_device.clone() else { return };
        if row["is_own"].as_bool().unwrap_or(false) || sender.is_empty() || sender == me {
            return;
        }
        let Some(principal) = self.directory.principal_of_device(sender) else { return };
        if Some(principal) == self.directory.self_principal.as_deref() {
            return;
        }
        let Some(identity) = self.directory.identities.get(principal) else { return };
        if identity.is_self {
            return;
        }
        let Some(target) =
            identity.devices.iter().find(|d| d.device_id == sender).and_then(|d| d.target.clone())
        else {
            return;
        };
        self.delivered.push(crate::receipts::Delivered {
            conversation_id: context.to_string(),
            message_id: message.to_owned(),
            receiver_device_id: me,
            sender_device_id: sender.to_owned(),
            sender_target: target,
        });
    }

    /// The received direct messages listed since the last call.
    pub fn take_delivered(&mut self) -> Vec<crate::receipts::Delivered> {
        std::mem::take(&mut self.delivered)
    }

    /// The participant a sending device stands for.
    pub(super) fn origin(&self, sender: &str) -> ParticipantId {
        let own = self.self_device.as_deref() == Some(sender);
        let principal = if own {
            self.directory.self_principal.as_deref()
        } else {
            self.directory.principal_of_device(sender)
        };
        ParticipantId::new(principal.unwrap_or(sender))
    }

    /// Emits a stored message the first time, when its payload changed, or
    /// when the view asked for its body again; remembers only a digest.
    pub(super) fn report_moment(&mut self, moment: Moment) -> Option<DomainEvent> {
        let is_message = matches!(moment.kind, MomentKind::Message(_));
        let digest = super::digest(&moment.kind);
        let wanted = self.resend.remove(&moment.id);
        let known = self.moments.get(&moment.id).copied();
        let event = match known {
            Some(seen) if seen.message && !is_message => return None,
            Some(seen) if seen.digest == digest && !wanted => return None,
            Some(_) => DomainEvent::MomentUpdated {
                moment_id: moment.id.clone(),
                lifecycle: Some(moment.lifecycle),
                kind: Some(moment.kind.clone()),
            },
            None => DomainEvent::MomentAdded(Box::new(moment.clone())),
        };
        self.moments.insert(moment.id, super::Reported { digest, message: is_message });
        Some(event)
    }

    fn message_moment(&self, context: &ContextId, row: &Value, now: Timestamp) -> Moment {
        let message = text(row, "id");
        let own = row["is_own"].as_bool().unwrap_or(false);
        let origin = if own { self.own_origin() } else { self.origin(author_device(row)) };
        let kind = match text(row, "state") {
            "tombstone" => {
                MomentKind::Message(MessagePayload { tombstone: true, ..Default::default() })
            }
            "content" => {
                let body = row["text"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| {
                        self.plaintext.get(&MomentId::new(message)).map(ToString::to_string)
                    })
                    .unwrap_or_default();
                MomentKind::Message(MessagePayload {
                    text: body,
                    receipt: if own { receipt(row) } else { Receipt::default() },
                    attachment: self.attachment_of(message),
                    ..Default::default()
                })
            }
            _ => MomentKind::Fallback(FallbackPayload {
                reason: "backend.message.unsupported_kind".to_owned(),
                text: String::new(),
                unsupported: Some(UnsupportedInfo {
                    kind: row["unsupported"]["kind"].to_string(),
                    layer: row["unsupported"]["layer"].to_string(),
                    reason: text(&row["unsupported"], "reason").to_owned(),
                    raw_length: row["unsupported"]["raw_length"].as_u64().unwrap_or(0),
                }),
            }),
        };
        Moment {
            order: None,
            id: message_moment_id(context, message),
            context_id: context.clone(),
            origin,
            at: row["hlc_ms"].as_u64().map_or(now, |ms| Timestamp(ms.try_into().unwrap_or(0))),
            kind,
            lifecycle: Lifecycle::Done,
        }
    }
}

/// The device that wrote a read-model row, or empty.
fn author_device(row: &Value) -> &str {
    text(&row["author"], "device_id")
}

/// The best receipt any recipient device reported for an own message.
fn receipt(row: &Value) -> Receipt {
    let states: Vec<&str> = list(row, "receipts").iter().map(|r| text(r, "state")).collect();
    if states.contains(&"read") {
        Receipt::Read
    } else if states.contains(&"delivered") {
        Receipt::Delivered
    } else {
        Receipt::Sent
    }
}
