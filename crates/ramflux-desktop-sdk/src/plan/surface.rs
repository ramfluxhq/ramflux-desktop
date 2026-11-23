// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Surface effects: a recipient's request for the author's current state of
//! an out-of-sync surface (`surface.request_resync`; the daemon rate-limits
//! it and the author's device answers over a pair route). It names the
//! surface by the daemon's scoped `surface_ref`, which is the desktop's
//! surface id; afterwards the surface is read again. Acting on a surface
//! needs the card runtime and is unsupported (see `capabilities`).

use serde_json::{Value, json};

use super::fetch::Fetch;
use super::{Known, Plan, refuse_invalid, text};
use crate::bus::BusRequest;

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
