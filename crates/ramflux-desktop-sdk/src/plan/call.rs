// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Voice call effects (CALL-01 §6): `call.invite {conversation_id}` and
//! `call.answer` / `call.reject` / `call.hangup {call_id}`. Each answers
//! with the call row, which the translator reports at once; later changes
//! arrive as `call.changed` events. Nothing is read back afterwards.
//!
//! The media session (`call.media.session`) is not an effect: the media
//! engine asks for it through the source, and its keys never reach the
//! translator (see [`crate::bridge`]).

use ramflux_desktop_core::methods;
use serde_json::{Value, json};

use super::{Plan, call, refuse_invalid, text};
use crate::bus::BusRequest;

/// The `sdk_api` label of call requests.
pub const API: &str = "call";

/// Plans a call effect.
pub fn call_effect(acct: Option<&str>, method: &str, body: &Value) -> Plan {
    match method {
        methods::CALL_INVITE => {
            let conversation = text(body, "context_id");
            if conversation.is_empty() {
                return refuse_invalid("a call needs a conversation");
            }
            call(BusRequest::new(
                acct,
                API,
                "call.invite",
                json!({ "conversation_id": conversation }),
            ))
        }
        methods::CALL_ANSWER | methods::CALL_REJECT | methods::CALL_HANGUP => {
            let call_id = text(body, "call_id");
            if call_id.is_empty() {
                return refuse_invalid("no call named");
            }
            call(BusRequest::new(acct, API, method, json!({ "call_id": call_id })))
        }
        _ => refuse_invalid("no daemon counterpart"),
    }
}

/// The media session request of `call_id`.
pub fn media_session(acct: &str, call_id: &str) -> BusRequest {
    BusRequest::new(Some(acct), API, "call.media.session", json!({ "call_id": call_id }))
}
