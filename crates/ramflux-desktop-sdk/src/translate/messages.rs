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
    ContextId, DomainEvent, FallbackPayload, Lifecycle, MessagePayload, Moment, MomentId,
    MomentKind, ParticipantId, Receipt, Timestamp, UnsupportedInfo,
};
use serde_json::Value;

use super::conversation::message_moment_id;
use super::{Translated, Translator, list, text};
use crate::plan::fetch::Fetch;

/// The contract every read-model page carries.
pub(crate) const READ_MODEL_SCHEMA: &str = "ramflux.message.read_model.v1";

impl Translator {
    /// Remembers the text of a message this device sent, shown until the
    /// read model lists it.
    pub(super) fn remember_sent(&mut self, message_id: &str, body: &str) {
        self.plaintext.insert(&MomentId::new(message_id), body.to_owned());
    }

    /// One read-model page of a known context (its newest page, or an
    /// older one). A page of another schema is not read. The page's
    /// snapshot becomes the conversation's change cursor; its `next_before`
    /// is where older history continues (the newest page sets it only
    /// while no older page was read).
    pub(super) fn messages(
        &mut self,
        context: &ContextId,
        fetch: &Fetch,
        value: &Value,
        now: Timestamp,
    ) -> Translated {
        let older = matches!(fetch, Fetch::MessagesBefore(..));
        if older {
            self.older_reading.remove(context);
        }
        if !self.contexts.contains_key(context) || text(value, "schema") != READ_MODEL_SCHEMA {
            return Translated::default();
        }
        if let Some(snapshot) = value["snapshot_sequence"].as_u64() {
            let cursor = self.read_cursors.entry(context.clone()).or_insert(snapshot);
            *cursor = (*cursor).max(snapshot);
        }
        let next = match &value["next_before"] {
            Value::Null => None,
            before => Some(before.to_string()),
        };
        if older || !self.older.contains_key(context) {
            self.older.insert(context.clone(), next);
        }
        // Oldest first, so moments are added in the order they were written.
        let rows: Vec<Value> = list(value, "messages").iter().rev().cloned().collect();
        Translated { events: self.message_rows(context, &rows, now), follow: Vec::new() }
    }

    /// The rows of a known context changed after its cursor, oldest change
    /// first; the cursor moves to `next_after`, and while `more` remains
    /// the next changes are read.
    pub(super) fn message_changes(
        &mut self,
        context: &ContextId,
        value: &Value,
        now: Timestamp,
    ) -> Translated {
        if !self.contexts.contains_key(context) || text(value, "schema") != READ_MODEL_SCHEMA {
            return Translated::default();
        }
        let rows: Vec<Value> = list(value, "changes").to_vec();
        let events = self.message_rows(context, &rows, now);
        let mut follow = Vec::new();
        if let Some(after) = value["next_after"].as_u64() {
            let cursor = self.read_cursors.entry(context.clone()).or_insert(after);
            *cursor = (*cursor).max(after);
            if value["more"].as_bool().unwrap_or(false) {
                follow.push(Fetch::MessageChanges(context.clone(), *cursor));
            }
        }
        Translated { events, follow }
    }

    /// The read of the page older than what is shown of `context`, unless
    /// its beginning was read, no page was read yet, or one is on its way.
    pub fn older_read(&mut self, context: &ContextId) -> Option<Fetch> {
        let before = self.older.get(context)?.clone()?;
        self.older_reading
            .insert(context.clone())
            .then(|| Fetch::MessagesBefore(context.clone(), before))
    }

    /// The older page of `context` could not be read: it may be asked for
    /// again.
    pub fn older_failed(&mut self, context: &ContextId) {
        self.older_reading.remove(context);
    }

    /// The change cursor of `context` is no longer usable (pruned, or
    /// ahead of the journal): its newest page is read again.
    pub fn forget_read_cursor(&mut self, context: &ContextId) {
        self.read_cursors.remove(context);
        self.older.remove(context);
        self.older_reading.remove(context);
    }

    /// Projects read-model rows (oldest first) of a known context.
    fn message_rows(
        &mut self,
        context: &ContextId,
        rows: &[Value],
        now: Timestamp,
    ) -> Vec<DomainEvent> {
        if !self.contexts.contains_key(context) {
            return Vec::new();
        }
        let mut events = Vec::new();
        for row in rows {
            let state = text(row, "state");
            if text(row, "id").is_empty() {
                continue;
            }
            // Erased on this device only (a delete for me): the row leaves
            // the conversation here, and no other device is changed.
            if state == "tombstone" && row["erasure"]["scope"].as_str() == Some("local") {
                let moment_id = message_moment_id(context, self.local_id(text(row, "id")));
                self.message_targets.remove(&moment_id);
                if self.moments.remove(&moment_id).is_some() {
                    events.push(DomainEvent::MomentRemoved { moment_id });
                }
                continue;
            }
            if state == "content" && text(row, "kind") != "message" {
                continue;
            }
            // An own row the daemon lists before it answered the send is
            // that send: it is shown once its id is known (the send's
            // refresh reads it again), never as a second message.
            let id = text(row, "id");
            if row["is_own"].as_bool().unwrap_or(false)
                && !self.sending.is_empty()
                && !self.local_ids.contains_key(id)
                && !self.moments.contains_key(&message_moment_id(context, id))
            {
                continue;
            }
            // The newest content row a peer wrote is the conversation's read
            // mark (by its clock: a page of older history never moves it
            // back). This account's own rows never are: a read receipt for
            // them reaches no peer.
            let peer = !row["is_own"].as_bool().unwrap_or(false);
            if state == "content" && peer {
                let at = row["hlc_ms"].as_u64().unwrap_or(0);
                if self.newest_peer.get(context).is_none_or(|(seen, _)| at >= *seen) {
                    self.newest_peer.insert(context.clone(), (at, id.to_owned()));
                }
            }
            let (moment, objects) = self.message_moment(context, row, now);
            events.extend(objects);
            events.extend(self.report_moment(moment));
        }
        events
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

    fn message_moment(
        &mut self,
        context: &ContextId,
        row: &Value,
        now: Timestamp,
    ) -> (Moment, Vec<DomainEvent>) {
        // A message this device sent keeps the moment it was shown as.
        let daemon_id = text(row, "id").to_owned();
        let message = self.local_id(&daemon_id).to_owned();
        let own = row["is_own"].as_bool().unwrap_or(false);
        let (media, objects) = if text(row, "state") == "content" {
            self.row_media(&message, &daemon_id, own, row)
        } else {
            (Vec::new(), Vec::new())
        };
        let message = message.as_str();
        let moment_id = message_moment_id(context, message);
        if !daemon_id.is_empty() {
            self.message_targets.insert(
                moment_id.clone(),
                crate::plan::message::MessageTarget {
                    daemon_id: daemon_id.clone(),
                    revision: row["revision"].as_u64().unwrap_or(0),
                },
            );
        }
        let reply_to = reply_target(row).map(|target| ramflux_desktop_core::ReplyRef {
            message_id: message_moment_id(context, self.local_id(&target)),
        });
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
                    media,
                    revision: row["revision"].as_u64().unwrap_or(0),
                    reply_to,
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
        let moment = Moment {
            order: None,
            id: moment_id,
            context_id: context.clone(),
            origin,
            at: row["hlc_ms"].as_u64().map_or(now, |ms| Timestamp(ms.try_into().unwrap_or(0))),
            kind,
            lifecycle: Lifecycle::Done,
        };
        (moment, objects)
    }
}

/// The daemon id of the message a read-model row replies to (its typed
/// content's `reply_to`), base64url like every read-model id.
fn reply_target(row: &Value) -> Option<String> {
    if row["content"].is_null() {
        return None;
    }
    let content: ramflux_protocol::message::Content =
        serde_json::from_value(row["content"].clone()).ok()?;
    match content.value? {
        ramflux_protocol::message::content::Value::Message(message) => message
            .reply_to
            .filter(|target| !target.id.is_empty())
            .map(|target| ramflux_protocol::encode_base64url(&target.id)),
        _ => None,
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

impl Translator {
    /// The read receipts due: the conversation shown (focused) is marked
    /// read through its newest message a peer wrote, once per new mark
    /// (`message.receipt.read` `{conversation_id, message_id, reader_id}`).
    /// The SDK routes the receipt to the author's device, so the peer sees
    /// the message read. Own messages are never the mark: a receipt for
    /// one would only move the local read mark.
    pub fn take_reads(&mut self) -> Vec<crate::bus::BusRequest> {
        let (Some(context), Some(reader)) = (self.focus.clone(), self.self_device.clone()) else {
            return Vec::new();
        };
        let Some((_, newest)) = self.newest_peer.get(&context).cloned() else {
            return Vec::new();
        };
        if self.read_sent.get(&context) == Some(&newest) {
            return Vec::new();
        }
        let Some(account) = self.scope.account().map(|a| a.to_string()) else {
            return Vec::new();
        };
        self.read_sent.insert(context.clone(), newest.clone());
        let body = serde_json::json!({
            "conversation_id": context,
            "message_id": newest,
            "reader_id": reader,
        });
        let key = format!(
            "desk-read-{}",
            ramflux_protocol::hash_hex(
                "ramflux.desktop.read",
                format!("{context}\n{newest}").as_bytes()
            )
        );
        vec![
            crate::bus::BusRequest::new(
                Some(account.as_str()),
                "message",
                "message.receipt.read",
                body,
            )
            .with_operation_id(key.chars().take(64).collect()),
        ]
    }
}
