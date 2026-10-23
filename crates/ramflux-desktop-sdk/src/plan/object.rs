// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Objects of the object directory: open, save, resume.
//!
//! - `object.get` (pull): `object.get` for the directory's object id. The
//!   daemon decrypts the stored object; the bytes are dropped (wiped) once
//!   the answer arrived, which shows the object can be opened.
//! - `object.save`: the same read, the bytes written to the path the user
//!   chose, then wiped. Bytes never reach the translator or the store.
//! - `object.transfer.resume`: `object.transfer.resume` in the object's
//!   direction (upload for an outgoing object, download for an incoming
//!   one) through the account's node relay.
//!
//! One `object.get` answer carries the whole object in one frame, so the
//! daemon refuses objects above its one-shot limit; that refusal is shown.

use ramflux_desktop_core::{Direction, ObjectId, methods};
use ramflux_sdk::{LocalBusObjectGetRequest, LocalBusObjectTransferResumeRequest};
use serde_json::Value;

use super::fetch::{Fetch, Paged};
use super::{Known, Plan, refuse, refuse_invalid, text};
use crate::bus::BusRequest;
use crate::translate::reason::key;

/// Plans an object effect.
pub(super) fn object_call(
    acct: Option<&str>,
    method: &str,
    body: &Value,
    known: &impl Known,
) -> Plan {
    let object = ObjectId::new(text(body, "object_id"));
    if object.as_str().is_empty() {
        return refuse_invalid("no object named");
    }
    match method {
        methods::OBJECT_RETRY => {
            let Some(directory) = known.directory() else { return refuse(key::NO_DIRECTION) };
            let direction = match directory.objects.get(&object) {
                Some(Direction::Outbound) => "upload",
                Some(Direction::Inbound) => "download",
                None => return refuse(key::NO_DIRECTION),
            };
            let resume = LocalBusObjectTransferResumeRequest {
                object_id: object.to_string(),
                direction: direction.to_owned(),
                relay_endpoint: directory.relay_endpoint.clone(),
                relay_service_key_base64: None,
                relay_interrupt_after_chunks: None,
            };
            match serde_json::to_value(resume) {
                Ok(body) => Plan::Call {
                    request: BusRequest::new(acct, "object", "object.transfer.resume", body),
                    refresh: vec![Fetch::first(Paged::Objects)],
                },
                Err(error) => refuse_invalid(&error.to_string()),
            }
        }
        _ => {
            let get = LocalBusObjectGetRequest {
                object_id: object.to_string(),
                relay_endpoint: None,
                relay_service_key_base64: None,
                relay_ack: false,
                relay_interrupt_after_chunks: None,
            };
            let save_to = (method == methods::OBJECT_SAVE).then(|| text(body, "path").to_owned());
            if save_to.as_deref().is_some_and(str::is_empty) {
                return refuse_invalid("no path chosen");
            }
            match serde_json::to_value(get) {
                Ok(body) => Plan::Open {
                    request: BusRequest::new(acct, "object", "object.get", body),
                    save_to,
                },
                Err(error) => refuse_invalid(&error.to_string()),
            }
        }
    }
}
