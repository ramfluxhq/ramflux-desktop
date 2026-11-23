// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! What this device sends that the daemon has not listed yet: a new
//! conversation (an attachment on its way: see `objects`).
//!
//! - A started conversation is shown as soon as the daemon accepted its
//!   first message; its `conversation.directory` row follows with the next
//!   read and replaces it. Until the directory lists it once, a listing
//!   without it (read before the start) does not remove it.

use std::collections::BTreeSet;

use ramflux_desktop_core::{
    Context, ContextId, ContextKind, DomainEvent, Lifecycle, MessagePayload, Moment, MomentKind,
    ParticipantId, Receipt, Timestamp,
};
use serde_json::Value;

use super::conversation::message_moment_id;
use super::{Translator, text};
use crate::directory::field;

impl Translator {
    /// Projects a locally submitted text message using its final daemon identity.
    pub(crate) fn outgoing_message(
        &mut self,
        body: &Value,
        message: &str,
        receipt: Receipt,
        at: Timestamp,
    ) -> Vec<DomainEvent> {
        let context = ContextId::new(text(body, "context_id"));
        self.remember_sent(message, text(body, "text"));
        if matches!(receipt, Receipt::Sending | Receipt::WaitingForNode { .. }) {
            self.sending.insert(message.to_owned());
        } else {
            self.sending.remove(message);
        }
        let moment = Moment {
            order: None,
            id: message_moment_id(&context, message),
            context_id: context,
            origin: self.own_origin(),
            at,
            lifecycle: match receipt {
                Receipt::Failed { .. } => Lifecycle::Failed,
                Receipt::Sent => Lifecycle::Done,
                _ => Lifecycle::InProgress,
            },
            kind: MomentKind::Message(MessagePayload {
                text: text(body, "text").to_owned(),
                receipt,
                tombstone: false,
                attachment: None,
                franking: None,
                ..Default::default()
            }),
        };
        self.report_moment(moment).into_iter().collect()
    }

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
            last_activity: None,
            preview: None,
        };
        self.put_context(context)
    }

    /// The daemon answered a message this device sent (`local`) with its
    /// own id (`daemon`): the read model's row of `daemon` is that moment.
    pub(crate) fn sent_as(&mut self, local: &str, daemon: &str) {
        if !local.is_empty() && !daemon.is_empty() && local != daemon {
            self.local_ids.insert(daemon.to_owned(), local.to_owned());
        }
    }

    /// The id this device gave a message it sent, else `message` itself.
    pub(super) fn local_id<'a>(&'a self, message: &'a str) -> &'a str {
        self.local_ids.get(message).map_or(message, String::as_str)
    }

    /// This account as the origin of what it sends.
    pub(super) fn own_origin(&self) -> ParticipantId {
        let own = self.directory.self_principal.as_deref().or(self.self_device.as_deref());
        ParticipantId::new(own.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

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
