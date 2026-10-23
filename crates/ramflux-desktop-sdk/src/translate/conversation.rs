// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Conversations, groups and their messages.

use std::collections::BTreeMap;

use ramflux_desktop_core::{
    Context, ContextId, ContextKind, DomainEvent, FallbackPayload, Lifecycle, MessagePayload,
    Moment, MomentId, MomentKind, ParticipantId, Receipt, Role, Timestamp,
};
use serde_json::Value;

use super::{Translated, Translator, list, seconds, text};
use crate::plan::fetch::Fetch;

/// Moment id of a stored message.
pub(crate) fn message_moment_id(context: &ContextId, message: &str) -> MomentId {
    MomentId::new(format!("msg:{context}:{message}"))
}

fn role(name: &str) -> Role {
    match name {
        "owner" => Role::Owner,
        "admin" => Role::Admin,
        "bot" => Role::Bot,
        _ => Role::Member,
    }
}

impl Translator {
    fn new_context(&self, id: &ContextId, kind: ContextKind) -> Context {
        Context {
            id: id.clone(),
            kind,
            // The daemon lists no titles; the id is shown.
            title: id.to_string(),
            participants: self.self_device.iter().map(ParticipantId::new).collect(),
            // Pins are restored in pin order once the lists are known.
            pinned: false,
            unread: 0,
            muted: false,
            disappearing_secs: None,
            roles: BTreeMap::new(),
            epoch: None,
        }
    }

    /// Emits the context when it differs from what was reported.
    pub(super) fn put_context(&mut self, context: Context) -> Vec<DomainEvent> {
        if self.contexts.get(&context.id) == Some(&context) {
            return Vec::new();
        }
        self.contexts.insert(context.id.clone(), context.clone());
        vec![DomainEvent::ContextUpserted(context)]
    }

    fn remove_context(&mut self, id: &ContextId) -> DomainEvent {
        // A removed context keeps its pin: should it come back, it is
        // restored pinned.
        self.contexts.remove(id);
        self.moments.retain(|_, m| &m.context_id != id);
        DomainEvent::ContextRemoved { context_id: id.clone() }
    }

    pub(super) fn conversation_list(&mut self, value: &Value) -> Translated {
        let mut out = Translated::default();
        let mut listed = Vec::new();
        for row in list(value, "conversations") {
            let id = ContextId::new(text(row, "conversation_id"));
            if row["is_archived"].as_bool().unwrap_or(false) || self.context_kind_is_group(&id) {
                continue;
            }
            // A conversation the daemon reports pinned is adopted only
            // while the desktop has no pins of its own recorded.
            if !self.pins_recorded
                && !row["pin_order"].is_null()
                && !self.contexts.contains_key(&id)
            {
                self.add_pin(&id);
            }
            let context = match self.contexts.get(&id) {
                Some(known) => known.clone(),
                None => self.new_context(&id, ContextKind::Direct),
            };
            out.events.extend(self.put_context(context));
            if self.pins.contains(&id) || self.focus.as_ref() == Some(&id) {
                out.follow.push(Fetch::Messages(id.clone()));
            }
            listed.push(id);
        }
        let gone: Vec<ContextId> = self
            .contexts
            .values()
            .filter(|c| c.kind == ContextKind::Direct && !listed.contains(&c.id))
            .map(|c| c.id.clone())
            .collect();
        for id in gone {
            out.events.push(self.remove_context(&id));
        }
        out
    }

    fn context_kind_is_group(&self, id: &ContextId) -> bool {
        self.contexts.get(id).is_some_and(|c| c.kind == ContextKind::Group)
    }

    pub(super) fn group_list(&mut self, value: &Value) -> Translated {
        let mut out = Translated::default();
        let mut listed = Vec::new();
        for group in list(value, "groups") {
            let id = ContextId::new(text(group, "group_id"));
            let mut context = match self.contexts.get(&id) {
                Some(known) if known.kind == ContextKind::Group => known.clone(),
                _ => self.new_context(&id, ContextKind::Group),
            };
            context.participants = list(group, "members")
                .iter()
                .filter_map(Value::as_str)
                .map(ParticipantId::new)
                .collect();
            context.roles = group["roles"]
                .as_object()
                .map(|roles| {
                    roles
                        .iter()
                        .filter_map(|(member, r)| {
                            r.as_str().map(|r| (ParticipantId::new(member.as_str()), role(r)))
                        })
                        .collect()
                })
                .unwrap_or_default();
            context.epoch = group["group_epoch"].as_u64();
            for member in context.participants.clone() {
                out.events.extend(self.participant(member.as_str(), member.as_str(), false));
            }
            out.events.extend(self.put_context(context));
            listed.push(id);
        }
        let gone: Vec<ContextId> = self
            .contexts
            .values()
            .filter(|c| c.kind == ContextKind::Group && !listed.contains(&c.id))
            .map(|c| c.id.clone())
            .collect();
        for id in gone {
            out.events.push(self.remove_context(&id));
        }
        out
    }

    /// Keeps decrypted bodies the daemon returned from a receive: a group's
    /// stored bodies are ciphertext, so its text is known only from here
    /// (or from what this device sent).
    pub(super) fn remember_plaintext(&mut self, value: &Value) {
        for message in list(value, "decrypted_messages") {
            let id = text(message, "message_id");
            if let Some(body) = message["body_utf8"].as_str()
                && !id.is_empty()
            {
                self.plaintext.insert(id.to_owned(), body.to_owned());
            }
        }
    }

    /// Remembers the text of a message this device sent.
    pub(super) fn remember_sent(&mut self, message_id: &str, body: &str) {
        self.plaintext.insert(message_id.to_owned(), body.to_owned());
    }

    pub(super) fn messages(
        &mut self,
        context: &ContextId,
        value: &Value,
        now: Timestamp,
    ) -> Vec<DomainEvent> {
        let mut events = Vec::new();
        let kind = self.contexts.get(context).map_or(ContextKind::Direct, |c| c.kind);
        if !self.contexts.contains_key(context) {
            let fresh = self.new_context(context, kind);
            events.extend(self.put_context(fresh));
        }
        for record in list(value, "messages") {
            let sender = text(record, "sender_id");
            if sender.is_empty() {
                continue;
            }
            events.extend(self.participant(sender, sender, false));
            if let Some(mut ctx) = self.contexts.get(context).cloned() {
                let sender_id = ParticipantId::new(sender);
                if !ctx.participants.contains(&sender_id) {
                    ctx.participants.push(sender_id);
                    events.extend(self.put_context(ctx));
                }
            }
            let moment = self.message_moment(context, kind, record, now);
            match self.moments.get(&moment.id) {
                Some(known) if known == &moment => {}
                Some(_) => {
                    events.push(DomainEvent::MomentUpdated {
                        moment_id: moment.id.clone(),
                        lifecycle: None,
                        kind: Some(moment.kind.clone()),
                    });
                    self.moments.insert(moment.id.clone(), moment);
                }
                None => {
                    events.push(DomainEvent::MomentAdded(Box::new(moment.clone())));
                    self.moments.insert(moment.id.clone(), moment);
                }
            }
        }
        events
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
        // A direct message is stored decrypted; a group message is stored as
        // ciphertext, so only a received or sent copy gives its text.
        let body = self
            .plaintext
            .get(message)
            .cloned()
            .or_else(|| (kind != ContextKind::Group).then_some(stored).flatten());
        let deleted = record["deleted"].as_bool().unwrap_or(false);
        let payload = match body {
            Some(text) => MomentKind::Message(MessagePayload {
                text,
                receipt: if outbound { receipt(record) } else { Receipt::default() },
                tombstone: deleted,
                attachment: None,
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
            origin: ParticipantId::new(sender),
            at: seconds(&record["created_at"]).unwrap_or(now),
            kind: payload,
            lifecycle: Lifecycle::Done,
        }
    }

    fn add_pin(&mut self, context: &ContextId) {
        if !self.pins.contains(context) {
            self.pins.push(context.clone());
            self.pins_changed = true;
        }
    }

    /// Applies a desktop-local pin (the daemon has no pin method) and
    /// returns the settings event.
    pub(super) fn local_pin(&mut self, context: &ContextId, pinned: bool) -> Vec<DomainEvent> {
        if !self.contexts.contains_key(context) {
            return Vec::new();
        }
        if pinned {
            self.add_pin(context);
        } else if self.pins.contains(context) {
            self.pins.retain(|id| id != context);
            self.pins_changed = true;
        }
        self.pins_recorded = true;
        self.show_pin(context, pinned)
    }

    /// Marks known, not yet pinned contexts of the persisted pins as pinned,
    /// in pin order. Called after the snapshot and when the lists change.
    pub fn restore_pins(&mut self) -> Vec<DomainEvent> {
        let due: Vec<ContextId> = self
            .pins
            .iter()
            .filter(|id| self.contexts.get(*id).is_some_and(|c| !c.pinned))
            .cloned()
            .collect();
        due.iter().flat_map(|id| self.show_pin(id, true)).collect()
    }

    fn show_pin(&mut self, context: &ContextId, pinned: bool) -> Vec<DomainEvent> {
        let Some(mut ctx) = self.contexts.get(context).cloned() else { return Vec::new() };
        ctx.pinned = pinned;
        self.contexts.insert(context.clone(), ctx);
        vec![DomainEvent::ContextSettingsChanged {
            context_id: context.clone(),
            pinned: Some(pinned),
            muted: None,
            disappearing: None,
        }]
    }

    /// Applies a confirmed context setting and returns its event.
    pub(super) fn context_setting(
        &mut self,
        context: &ContextId,
        muted: Option<bool>,
        disappearing: Option<ramflux_desktop_core::Disappearing>,
    ) -> Vec<DomainEvent> {
        let Some(mut ctx) = self.contexts.get(context).cloned() else { return Vec::new() };
        if let Some(muted) = muted {
            ctx.muted = muted;
        }
        if let Some(timer) = disappearing {
            ctx.disappearing_secs = match timer {
                ramflux_desktop_core::Disappearing::Off => None,
                ramflux_desktop_core::Disappearing::After(secs) => Some(secs),
            };
        }
        self.contexts.insert(context.clone(), ctx);
        vec![DomainEvent::ContextSettingsChanged {
            context_id: context.clone(),
            pinned: None,
            muted,
            disappearing,
        }]
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
