// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Stored messages of conversations and groups, as message moments.

use ramflux_desktop_core::{
    ContextId, ContextKind, DomainEvent, FallbackPayload, Lifecycle, MessagePayload, Moment,
    MomentId, MomentKind, ParticipantId, Receipt, Timestamp,
};
use serde_json::Value;

use super::conversation::message_moment_id;
use super::surfaces::is_surface_row;
use super::{Translated, Translator, list, seconds, text};

impl Translator {
    /// Keeps decrypted bodies the daemon returned from a receive: a group's
    /// stored bodies are ciphertext, so its text is known only from here
    /// (or from what this device sent).
    pub(super) fn remember_plaintext(&mut self, value: &Value) {
        for message in list(value, "decrypted_messages") {
            let id = text(message, "message_id");
            if let Some(body) = message["body_utf8"].as_str()
                && !id.is_empty()
            {
                self.plaintext.insert(&MomentId::new(id), body.to_owned());
            }
        }
    }

    /// Remembers the text of a message this device sent.
    pub(super) fn remember_sent(&mut self, message_id: &str, body: &str) {
        self.plaintext.insert(&MomentId::new(message_id), body.to_owned());
    }

    /// Stored messages of a known context. Senders are device ids: a
    /// message's origin is the sender's principal when the directories name
    /// it (this account's principal for this device), else the device id as
    /// the daemon reported it. Senders never become context participants;
    /// the conversation directory lists those.
    pub(super) fn messages(
        &mut self,
        context: &ContextId,
        value: &Value,
        now: Timestamp,
    ) -> Translated {
        let Some(kind) = self.contexts.get(context).map(|c| c.kind) else {
            return Translated::default();
        };
        let mut events = Vec::new();
        let mut surface_rows = Vec::new();
        for record in list(value, "messages") {
            if text(record, "sender_id").is_empty() {
                continue;
            }
            // A surface message is shown as its surface (one moment per
            // surface, however many versions), never as message text.
            if is_surface_row(record) {
                surface_rows.push(record);
                continue;
            }
            if kind == ContextKind::Direct {
                self.note_received(context, record);
            }
            let moment = self.message_moment(context, kind, record, now);
            events.extend(self.report_moment(moment));
        }
        let follow = self.surface_rows(context, &surface_rows);
        Translated { events, follow }
    }

    /// Notes a listed direct message from a peer, for its delivery receipt:
    /// never this device's own, nor one from this account's principal or
    /// its other devices, nor a deleted one. A received row names its
    /// sender by principal (this device's own rows name the device), so the
    /// receipt goes to that peer's first verified device with a delivery
    /// target (the row does not say which device sent it); a row naming a
    /// device sends to that device. Skipped while no target is listed.
    fn note_received(&mut self, context: &ContextId, record: &Value) {
        let (message, sender) = (text(record, "message_id"), text(record, "sender_id"));
        let Some(me) = self.self_device.clone() else { return };
        if message.is_empty() || sender == me || record["deleted"].as_bool().unwrap_or(false) {
            return;
        }
        let principal = match self.directory.principal_of_device(sender) {
            Some(principal) => principal,
            None if self.directory.identities.contains_key(sender) => sender,
            None => return,
        };
        if Some(principal) == self.directory.self_principal.as_deref() {
            return;
        }
        let Some(identity) = self.directory.identities.get(principal) else { return };
        if identity.is_self {
            return;
        }
        let device = if sender == principal {
            identity.devices.iter().find(|d| d.routable())
        } else {
            identity.devices.iter().find(|d| d.device_id == sender)
        };
        let Some((device, Some(target))) = device.map(|d| (d.device_id.clone(), d.target.clone()))
        else {
            return;
        };
        self.delivered.push(crate::receipts::Delivered {
            conversation_id: context.to_string(),
            message_id: message.to_owned(),
            receiver_device_id: me,
            sender_device_id: device,
            sender_target: target,
        });
    }

    /// The received direct messages listed since the last call.
    pub fn take_delivered(&mut self) -> Vec<crate::receipts::Delivered> {
        std::mem::take(&mut self.delivered)
    }

    /// The participant a sending device stands for.
    fn origin(&self, sender: &str) -> ParticipantId {
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
    fn report_moment(&mut self, moment: Moment) -> Option<DomainEvent> {
        let is_message = matches!(moment.kind, MomentKind::Message(_));
        let digest = super::digest(&moment.kind);
        let wanted = self.resend.remove(&moment.id);
        let known = self.moments.get(&moment.id).copied();
        let event = match known {
            // A body that is no longer in memory here (an evicted group
            // plaintext) never replaces the message already shown.
            Some(seen) if seen.message && !is_message => return None,
            Some(seen) if seen.digest == digest && !wanted => return None,
            Some(_) => DomainEvent::MomentUpdated {
                moment_id: moment.id.clone(),
                lifecycle: None,
                kind: Some(moment.kind.clone()),
            },
            None => DomainEvent::MomentAdded(Box::new(moment.clone())),
        };
        self.moments.insert(moment.id, super::Reported { digest, message: is_message });
        Some(event)
    }

    fn message_moment(
        &self,
        context: &ContextId,
        kind: ContextKind,
        record: &Value,
        now: Timestamp,
    ) -> Moment {
        let message = text(record, "message_id");
        let sender = text(record, "sender_id");
        let outbound = self.self_device.as_deref() == Some(sender);
        let stored: Option<String> = record["encrypted_body"].as_array().and_then(|bytes| {
            let bytes: Option<Vec<u8>> =
                bytes.iter().map(|b| b.as_u64().and_then(|v| u8::try_from(v).ok())).collect();
            bytes.and_then(|b| String::from_utf8(b).ok())
        });
        // A direct message row carries its text as `body_utf8` (an older
        // daemon stored it as the body bytes); a group message is stored as
        // ciphertext, so only a received or sent copy gives its text.
        let direct = (kind != ContextKind::Group)
            .then(|| record["body_utf8"].as_str().map(str::to_owned).or(stored))
            .flatten();
        let body = self.plaintext.get(&MomentId::new(message)).map(ToString::to_string).or(direct);
        let deleted = record["deleted"].as_bool().unwrap_or(false);
        let payload = match body {
            Some(text) => MomentKind::Message(MessagePayload {
                text,
                receipt: if outbound { receipt(record) } else { Receipt::default() },
                tombstone: deleted,
                attachment: self.attachment_of(message),
                franking: None,
            }),
            None if deleted => MomentKind::Message(MessagePayload {
                text: String::new(),
                receipt: Receipt::default(),
                tombstone: true,
                attachment: None,
                franking: None,
            }),
            None => MomentKind::Fallback(FallbackPayload {
                reason: "backend.message.encrypted_at_rest".to_owned(),
                text: String::new(),
            }),
        };
        Moment {
            id: message_moment_id(context, message),
            context_id: context.clone(),
            origin: self.origin(sender),
            at: seconds(&record["created_at"]).unwrap_or(now),
            kind: payload,
            lifecycle: Lifecycle::Done,
        }
    }
}

fn receipt(record: &Value) -> Receipt {
    let states: Vec<&str> = list(record, "receipts").iter().map(|r| text(r, "state")).collect();
    if states.contains(&"read") {
        Receipt::Read
    } else if states.contains(&"delivered") {
        Receipt::Delivered
    } else {
        Receipt::Sent
    }
}
