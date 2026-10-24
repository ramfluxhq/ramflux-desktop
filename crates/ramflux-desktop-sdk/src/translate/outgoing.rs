// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! What this device sends that the daemon has not listed yet: a new
//! conversation, and an attachment on its way.
//!
//! - A started conversation is shown as soon as the daemon accepted its
//!   first message; its `conversation.directory` row follows with the next
//!   read and replaces it. Until the directory lists it once, a listing
//!   without it (read before the start) does not remove it.
//! - An attachment is one `message.submit` carrying the file. While that
//!   request runs, the conversation shows the message at once (its own
//!   moment id, receipt "sending", in progress) with the object as its
//!   attachment, transferring: the daemon reports no bytes moved, so no
//!   percentage is claimed. A moment keeps its kind for life, so the
//!   transfer is shown on the message's attachment rather than as a
//!   separate transfer moment; when the daemon lists the message it updates
//!   the same moment (receipts included). A failed send marks the message
//!   and its object failed, with the reason.

use std::collections::BTreeSet;

use ramflux_desktop_core::{
    Context, ContextId, ContextKind, Direction, DomainEvent, Lifecycle, MessagePayload, Moment,
    MomentId, MomentKind, Object, ObjectId, ParticipantId, Receipt, Timestamp, TransferState,
};
use serde_json::{Value, json};

use super::conversation::message_moment_id;
use super::{Reported, Translator, digest, list, text};
use crate::directory::field;

/// Fields the bridge adds to an attach effect's body to find its message
/// and object again when the answer arrives.
const SENDING_MOMENT: &str = "sending_moment";
const SENDING_OBJECT: &str = "sending_object";

/// Bytes encoded by an unpadded (or padded) base64 string.
fn decoded_len(encoded: &str) -> u64 {
    let digits = encoded.trim_end_matches('=').len() as u64;
    digits * 3 / 4
}

impl Translator {
    /// A conversation this device started (its first message was accepted).
    pub(super) fn conversation_started(
        &mut self,
        body: &Value,
        answer: &Value,
    ) -> Vec<DomainEvent> {
        let id = ContextId::new(text(body, "context_id"));
        if id.as_str().is_empty() || self.contexts.contains_key(&id) {
            return Vec::new();
        }
        if let Some(message) = answer["message_id"].as_str() {
            self.remember_sent(message, text(body, "text"));
        }
        // The participants the directory will list: this account and the
        // contact.
        let mut participants: BTreeSet<String> =
            self.directory.self_principal.iter().cloned().collect();
        participants.insert(text(body, "participant_id").to_owned());
        self.started.insert(id.clone());
        let title = field(body, "title").unwrap_or_else(|| id.to_string());
        let context = Context {
            id,
            kind: ContextKind::Direct,
            title,
            participants: participants.iter().map(|p| ParticipantId::new(p.as_str())).collect(),
            pinned: false,
            unread: 0,
            muted: false,
            disappearing_secs: None,
            roles: std::collections::BTreeMap::new(),
            epoch: None,
        };
        self.put_context(context)
    }

    /// An attachment request is about to be sent (`request` is the
    /// `message.submit` body). Returns the message and object events, and
    /// adds their ids to `body` (the effect's).
    pub fn attachment_sending(
        &mut self,
        request: &Value,
        body: &mut Value,
        now: Timestamp,
    ) -> Vec<DomainEvent> {
        let context = ContextId::new(text(request, "conversation_id"));
        let message = text(request, "message_id");
        let Some(attachment) = list(request, "attachments").first() else { return Vec::new() };
        let object_id = ObjectId::new(text(attachment, "object_id"));
        if context.as_str().is_empty() || message.is_empty() || object_id.as_str().is_empty() {
            return Vec::new();
        }
        let object = Object {
            id: object_id.clone(),
            name: field(attachment, "name"),
            size: attachment["plaintext_base64"].as_str().map(decoded_len),
            direction: Direction::Outbound,
            transfer: TransferState::Transferring { transferred: 0 },
            saved_to: None,
            preview: None,
        };
        let id = message_moment_id(&context, message);
        let kind = MomentKind::Message(MessagePayload {
            text: String::new(),
            receipt: Receipt::Sending,
            tombstone: false,
            attachment: Some(object_id.clone()),
            franking: None,
        });
        self.moments.insert(id.clone(), Reported { digest: digest(&kind), message: true });
        self.sent_attachments.insert(message.to_owned(), object_id.clone());
        if let Value::Object(map) = body {
            map.insert(SENDING_MOMENT.to_owned(), json!(id));
            map.insert(SENDING_OBJECT.to_owned(), json!(object_id));
        }
        let origin = self.own_origin();
        let mut events = self.put_object(object);
        events.push(DomainEvent::MomentAdded(Box::new(Moment {
            id,
            context_id: context,
            origin,
            at: now,
            kind,
            lifecycle: Lifecycle::InProgress,
        })));
        events
    }

    /// The daemon accepted an attachment's message: the message is done
    /// (the object's state comes from the object directory next).
    pub(super) fn attachment_sent(&mut self, body: &Value) -> Vec<DomainEvent> {
        let id = MomentId::new(text(body, SENDING_MOMENT));
        if id.as_str().is_empty() {
            return Vec::new();
        }
        vec![DomainEvent::MomentUpdated {
            moment_id: id,
            lifecycle: Some(Lifecycle::Done),
            kind: None,
        }]
    }

    /// An attachment's message failed: the message and its object show the
    /// reason.
    pub(super) fn attachment_failed(&mut self, body: &Value, reason: &str) -> Vec<DomainEvent> {
        let (id, object_id) =
            (MomentId::new(text(body, SENDING_MOMENT)), ObjectId::new(text(body, SENDING_OBJECT)));
        if id.as_str().is_empty() {
            return Vec::new();
        }
        let mut events = Vec::new();
        if let Some(mut object) = self.objects.get(&object_id).cloned() {
            object.transfer = TransferState::Failed { reason: reason.to_owned() };
            events.extend(self.put_object(object));
        }
        events.push(DomainEvent::MomentUpdated {
            moment_id: id,
            lifecycle: Some(Lifecycle::Failed),
            kind: None,
        });
        events
    }

    /// The object a stored message carries: one this device sent with it,
    /// else the one the object directory links to the message.
    pub(super) fn attachment_of(&self, message: &str) -> Option<ObjectId> {
        self.sent_attachments.get(message).or_else(|| self.message_objects.get(message)).cloned()
    }

    /// This account as the origin of what it sends.
    fn own_origin(&self) -> ParticipantId {
        let own = self.directory.self_principal.as_deref().or(self.self_device.as_deref());
        ParticipantId::new(own.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoded_length_of_base64() {
        assert_eq!(decoded_len(&ramflux_protocol::encode_base64url([0_u8; 8192])), 8192);
        assert_eq!(decoded_len(&ramflux_protocol::encode_base64url([1_u8; 7])), 7);
        assert_eq!(decoded_len("AAE="), 2);
        assert_eq!(decoded_len(""), 0);
    }

    #[test]
    fn a_listing_read_before_the_start_does_not_remove_the_started_conversation() {
        use crate::plan::fetch::{Fetch, Paged};
        let mut translator = Translator::new(
            super::super::Scope::Account(ramflux_desktop_core::AccountId::new("a")),
            "s",
        );
        let listing = |ids: &[&str]| {
            let rows: Vec<Value> = ids
                .iter()
                .map(|id| json!({ "id": id, "kind": "direct", "participants": [] }))
                .collect();
            json!({ "conversations": rows, "next_cursor": null })
        };
        let read = Fetch::first(Paged::Conversations);
        let removed = |events: &[DomainEvent]| {
            events.iter().any(|e| matches!(e, DomainEvent::ContextRemoved { .. }))
        };
        let _ = translator.fetched(&read, &listing(&["old"]), Timestamp(0));
        let body = json!({ "context_id": "new", "participant_id": "p", "text": "hi" });
        let shown = translator.conversation_started(&body, &json!({ "message_id": "m" }));
        assert!(!shown.is_empty());
        // A listing that was read before the start.
        let stale = translator.fetched(&read, &listing(&["old"]), Timestamp(0));
        assert!(!removed(&stale.events), "kept until the directory lists it");
        let _ = translator.fetched(&read, &listing(&["old", "new"]), Timestamp(0));
        // Once listed, it follows the directory like any other.
        let gone = translator.fetched(&read, &listing(&["old"]), Timestamp(0));
        assert!(removed(&gone.events));
    }
}
