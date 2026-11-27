// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Bus requests of the card runtime (`card.session.*`).
//!
//! The runtime renders a card; the desktop sends it host events and slot
//! replies and draws the batches it returns (see `crate::translate::card_session`).
//! Every method takes the account. Event and slot bodies carry user input and
//! runtime tokens: core hands them over as one secret JSON object, which this
//! module splits into secret fields (spliced into the frame only when it is
//! written, never logged). Only `submit` is a network call on the daemon, and
//! it is never retried.

use ramflux_desktop_core::{SecretInput, methods};
use serde_json::{Value, json};

use crate::bus::BusRequest;

/// The routing label of the card calls.
const SDK_API: &str = "surface";

/// Whether `method` is a card runtime call.
pub fn is_card_method(method: &str) -> bool {
    method.starts_with("card.session.")
}

/// The bus request of one card call, from the body core built and its
/// secrets. `None` when the call does not follow the card contract.
pub fn request(
    account: Option<&str>,
    method: &str,
    body: &Value,
    secrets: Vec<SecretInput>,
) -> Option<BusRequest> {
    let session = body["session"].as_u64();
    Some(match method {
        methods::CARD_SESSION_OPEN => BusRequest::new(
            account,
            SDK_API,
            "card.session.open",
            json!({
                "surface_ref": body["surface_id"].as_str()?,
                "locale": body["locale"].as_str()?,
                "dark": body["dark"].as_bool()?,
            }),
        ),
        methods::CARD_SESSION_UPDATE => session_call(account, "card.session.update", session?),
        methods::CARD_SESSION_PRESENT => BusRequest::new(
            account,
            SDK_API,
            "card.session.present",
            json!({
                "session": session?,
                "locale": body["locale"].as_str()?,
                "dark": body["dark"].as_bool()?,
            }),
        ),
        methods::CARD_SESSION_EVENT => {
            let secret = one_secret(secrets)?;
            let token = secret.get("token")?.clone();
            let event = secret.get("event")?.clone();
            BusRequest::new(account, SDK_API, "card.session.event", json!({ "session": session? }))
                .with_json("token", token)
                .with_json("event", event)
        }
        methods::CARD_SESSION_SLOT => {
            let secret = one_secret(secrets)?;
            let token = secret.get("token")?.clone();
            let slot_id = secret.get("slot_id")?.as_str()?.to_owned();
            let request_id = secret.get("request_id")?.as_str()?.to_owned();
            BusRequest::new(
                account,
                SDK_API,
                "card.session.slot",
                json!({ "session": session?, "outcome": body["outcome"].as_str()? }),
            )
            .with_json("token", token)
            .with_secret("slot_id", SecretInput::new(slot_id))
            .with_secret("request_id", SecretInput::new(request_id))
        }
        methods::CARD_SESSION_SUBMIT => session_call(account, "card.session.submit", session?),
        methods::CARD_SESSION_SUSPEND => session_call(account, "card.session.suspend", session?),
        methods::CARD_SESSION_RESUME => session_call(account, "card.session.resume", session?),
        methods::CARD_SESSION_DISPOSE => session_call(account, "card.session.dispose", session?),
        _ => return None,
    })
}

/// The one secret of a card call: a JSON object of token and fields. The
/// text is parsed and dropped; the values move into secret fields.
fn one_secret(mut secrets: Vec<SecretInput>) -> Option<Value> {
    if secrets.len() != 1 {
        return None;
    }
    let secret = secrets.pop()?;
    serde_json::from_str(secret.expose()).ok()
}

fn session_call(account: Option<&str>, method: &'static str, session: u64) -> BusRequest {
    BusRequest::new(account, SDK_API, method, json!({ "session": session }))
}
