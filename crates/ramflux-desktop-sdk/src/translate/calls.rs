// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Voice calls (CALL-01 §6): the daemon's call rows as `CallUpserted`, and
//! the media session as the secret `CallMedia`.
//!
//! A call row comes as the answer to a call effect and as the body of each
//! `call.changed` event (stream or push). Rows are reported once per
//! sequence number: a replayed or repeated row, or one older than the last
//! reported, changes nothing. A row whose conversation is not listed yet
//! waits for the conversation listing it calls for, then is reported.
//!
//! Row fields: `call_id`, `conversation_id`, `peer_id`, `direction`
//! (`outgoing` / `incoming`), `state` (`outgoing_ringing`,
//! `incoming_ringing`, `connecting`, `active`, `ended`), `end_reason`
//! (`hangup`, `rejected`, `busy`, `timeout`, `answered_elsewhere`,
//! `failed`), `seq`, `created_at`, `answered_at`, `ended_at` (Unix seconds).
//! A row missing what a call needs, or naming an unknown state, is dropped
//! (nothing is guessed).

use ramflux_desktop_core::{
    Call, CallDirection, CallEndReason, CallId, CallKey, CallMedia, CallState, ContextId,
    DomainEvent, ParticipantId,
};
use serde_json::Value;
use zeroize::{Zeroize as _, Zeroizing};

use super::{Translated, Translator, seconds, text};
use crate::plan::fetch::{Fetch, Paged};
use crate::translate::reason::{key, keyed};

fn end_reason(row: &Value) -> CallEndReason {
    match text(row, "end_reason") {
        "hangup" => CallEndReason::HungUp,
        "rejected" => CallEndReason::Rejected,
        "busy" => CallEndReason::Busy,
        "timeout" => CallEndReason::Timeout,
        "answered_elsewhere" => CallEndReason::AnsweredElsewhere,
        // An ended call whose reason this desktop does not know failed as
        // far as it can tell.
        _ => CallEndReason::Failed,
    }
}

fn state(row: &Value) -> Option<CallState> {
    Some(match text(row, "state") {
        "outgoing_ringing" => CallState::OutgoingRinging,
        "incoming_ringing" => CallState::IncomingRinging,
        "connecting" => CallState::Connecting,
        "active" => CallState::Active,
        "ended" => CallState::Ended(end_reason(row)),
        _ => return None,
    })
}

impl Translator {
    /// The call a row describes, as seen by this account.
    fn call_of(&self, row: &Value) -> Option<Call> {
        let id = text(row, "call_id");
        let conversation = text(row, "conversation_id");
        let peer = text(row, "peer_id");
        if id.is_empty() || conversation.is_empty() || peer.is_empty() {
            return None;
        }
        let direction = match text(row, "direction") {
            "outgoing" => CallDirection::Outgoing,
            "incoming" => CallDirection::Incoming,
            _ => return None,
        };
        let caller = match direction {
            CallDirection::Incoming => peer.to_owned(),
            CallDirection::Outgoing => self.directory.self_principal.clone()?,
        };
        Some(Call {
            id: CallId::new(id),
            context_id: ContextId::new(conversation),
            peer: ParticipantId::new(peer),
            caller: ParticipantId::new(caller),
            direction,
            state: state(row)?,
            version: row["seq"].as_u64()?,
            started_at: row.get("created_at").and_then(seconds)?,
            answered_at: row.get("answered_at").and_then(seconds),
            ended_at: row.get("ended_at").and_then(seconds),
        })
    }

    /// Translates one call row (an effect's answer or a `call.changed`
    /// body). A row for a conversation not listed yet waits for the
    /// listing it asks for.
    pub fn call_row(&mut self, row: &Value) -> Translated {
        let Some(call) = self.call_of(row) else { return Translated::default() };
        if self.reported_calls.get(&call.id).is_some_and(|seq| *seq >= call.version) {
            return Translated::default();
        }
        if !self.contexts.contains_key(&call.context_id) {
            let newer = self
                .pending_calls
                .get(&call.id)
                .is_none_or(|held| held["seq"].as_u64() < Some(call.version));
            if newer {
                self.pending_calls.insert(call.id.clone(), row.clone());
            }
            return Translated {
                events: Vec::new(),
                follow: vec![Fetch::first(Paged::Conversations)],
            };
        }
        self.reported_calls.insert(call.id.clone(), call.version);
        self.pending_calls.remove(&call.id);
        Translated::events(vec![DomainEvent::CallUpserted(call)])
    }

    /// Reports held call rows whose conversation is listed now.
    pub(super) fn listed_calls(&mut self) -> Vec<DomainEvent> {
        let ready: Vec<Value> = self
            .pending_calls
            .values()
            .filter(|row| self.contexts.contains_key(&ContextId::new(text(row, "conversation_id"))))
            .cloned()
            .collect();
        ready.iter().flat_map(|row| self.call_row(row).events).collect()
    }
}

/// Reads a `call.media.session` answer and wipes its secrets from it.
///
/// Answer fields: `relay.endpoint` (`host:port`), `relay.token` (base64url,
/// opaque), `sframe.cipher_suite`, `sframe.send` / `sframe.receive`, each
/// `{kid, base_key}` (base64url).
///
/// # Errors
///
/// A locale key (`backend.call.media`) when a field is missing or not
/// decodable.
pub fn call_media(call_id: &CallId, mut answer: Value) -> Result<CallMedia, String> {
    let media = parse_media(call_id, &answer);
    for path in [["relay", "token"], ["sframe", "send"], ["sframe", "receive"]] {
        if let Some(value) = answer.get_mut(path[0]).and_then(|v| v.get_mut(path[1])) {
            wipe_value(value);
        }
    }
    media.ok_or_else(|| keyed(key::CALL_MEDIA, &[]))
}

fn wipe_value(value: &mut Value) {
    match value {
        Value::String(text) => text.zeroize(),
        Value::Object(map) => map.values_mut().for_each(wipe_value),
        _ => {}
    }
}

fn decode(value: &Value) -> Option<Zeroizing<Vec<u8>>> {
    ramflux_protocol::decode_base64url(value.as_str()?).ok().map(Zeroizing::new)
}

fn direction_key(value: &Value) -> Option<CallKey> {
    let base_key = decode(&value["base_key"]).filter(|k| !k.is_empty())?;
    Some(CallKey::new(value["kid"].as_u64()?, base_key.to_vec()))
}

fn parse_media(call_id: &CallId, answer: &Value) -> Option<CallMedia> {
    let relay = &answer["relay"];
    let sframe = &answer["sframe"];
    let endpoint = text(relay, "endpoint");
    if endpoint.is_empty() {
        return None;
    }
    Some(CallMedia {
        call_id: call_id.clone(),
        endpoint: endpoint.to_owned(),
        token: decode(&relay["token"])?,
        suite: u16::try_from(sframe["cipher_suite"].as_u64()?).ok()?,
        send: direction_key(&sframe["send"])?,
        receive: direction_key(&sframe["receive"])?,
    })
}
