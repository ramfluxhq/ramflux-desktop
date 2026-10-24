// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Surface effects: an action on the version the user saw (`surface.action`)
//! and a recipient's request for the author's current state of an
//! out-of-sync surface (`surface.request_resync`; the daemon rate-limits it
//! and the author's device answers over a pair route). Both name the
//! surface by the daemon's scoped `surface_ref`, which is the desktop's
//! surface id; after either the surface is read again.

use serde_json::{Value, json};

use super::fetch::Fetch;
use super::{Known, Plan, refuse_invalid, text};
use crate::bus::BusRequest;
use crate::surface::action::{Shown, action_body};

/// `surface.action` with the protocol's action bytes.
pub fn action(acct: Option<&str>, body: &Value, known: &impl Known) -> Plan {
    let reference = text(body, "surface_id");
    let Some(target) = known.surface(reference) else {
        return refuse_invalid("unknown surface");
    };
    let shown = Shown {
        version: body["version"].as_u64().unwrap_or(0),
        surface_hash: text(body, "surface_hash"),
        action_id: text(body, "action_id"),
        values: &body["values"],
    };
    match action_body(target, &shown) {
        Ok(request) => Plan::Call {
            request: BusRequest::new(acct, "surface", "surface.action", request),
            refresh: vec![Fetch::Surface(reference.to_owned())],
        },
        Err(message) => refuse_invalid(&message),
    }
}

/// `surface.request_resync` for one surface.
pub fn request_resync(acct: Option<&str>, body: &Value, known: &impl Known) -> Plan {
    let reference = text(body, "surface_id");
    if known.surface(reference).is_none() {
        return refuse_invalid("unknown surface");
    }
    Plan::Call {
        request: BusRequest::new(
            acct,
            "surface",
            "surface.request_resync",
            json!({ "surface_ref": reference }),
        ),
        refresh: vec![Fetch::Surface(reference.to_owned())],
    }
}
