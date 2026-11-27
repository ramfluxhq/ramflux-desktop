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

use ramflux_desktop_core::{ContextId, SecretInput, methods};
use serde_json::{Value, json};

use super::Plan;
use super::fetch::Fetch;
use crate::bus::{BusRequest, wipe};

/// The routing label of the card calls.
const SDK_API: &str = "surface";

/// Whether `method` is a card runtime call, or the card approval the desktop
/// finishes after the runtime recorded the verdict.
pub fn is_card_method(method: &str) -> bool {
    method.starts_with("card.session.") || method == methods::CARD_APPROVAL_FINISH
}

/// The fields of the `task.resolve` body core builds for an approval.
const RESOLVE_FIELDS: [&str; 7] =
    ["conversation_id", "task_id", "step_id", "base_revision", "target", "decision_id", "decision"];

/// The two calls that finish an approval verdict: `grant.approve` or
/// `grant.deny` with the record the verdict names, then `task.resolve` with
/// the body core built, unchanged (its fields travel as secret fields; none
/// is read, merged or rebuilt). `None` when the call does not follow the
/// contract, a body with a field the task contract does not list included.
pub fn approval_finish(
    account: Option<&str>,
    body: &Value,
    secrets: Vec<SecretInput>,
) -> Option<Plan> {
    let mut secret = Parsed(one_secret(secrets)?);
    let request_ref = secret.text("request_ref")?;
    let mut resolve = Parsed(secret.take("task_resolve")?);
    let conversation = resolve.0["conversation_id"].as_str()?.to_owned();
    let task = resolve.0["task_id"].as_str()?.to_owned();
    let daemon = match body["outcome"].as_str()? {
        "confirm" => "grant.approve",
        "deny" => "grant.deny",
        _ => return None,
    };
    let grant = BusRequest::new(account, "grant", daemon, json!({ "approval_id": request_ref }));
    let mut resolved = BusRequest::new(account, "task", "task.resolve", json!({}));
    for field in RESOLVE_FIELDS {
        if let Some(value) = resolve.take(field) {
            resolved = resolved.with_json(field, value);
        }
    }
    if resolve.has_rest() {
        return None;
    }
    Some(Plan::Calls {
        requests: vec![grant, resolved],
        refresh: vec![Fetch::Grants, Fetch::Task(ContextId::new(conversation), task)],
    })
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
            let mut secret = Parsed(one_secret(secrets)?);
            let token = secret.take("token")?;
            let event = secret.take("event")?;
            BusRequest::new(account, SDK_API, "card.session.event", json!({ "session": session? }))
                .with_json("token", token)
                .with_json("event", event)
        }
        methods::CARD_SESSION_SLOT => {
            let mut secret = Parsed(one_secret(secrets)?);
            let token = secret.take("token")?;
            let slot_id = secret.text("slot_id")?;
            let request_id = secret.text("request_id")?;
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

/// The parsed secret of a card call. Fields move out of it into secret
/// fields; whatever is left (also on an early return) is wiped when it
/// drops.
struct Parsed(Value);

impl Parsed {
    /// Moves the field `name` out (`None` when it is absent or null).
    fn take(&mut self, name: &str) -> Option<Value> {
        self.0.get_mut(name).map(Value::take).filter(|value| !value.is_null())
    }

    /// Moves the text field `name` out.
    fn text(&mut self, name: &str) -> Option<String> {
        match self.take(name)? {
            Value::String(text) => Some(text),
            other => {
                wipe(other);
                None
            }
        }
    }
}

impl Parsed {
    /// Whether any field is left after the ones taken out.
    fn has_rest(&self) -> bool {
        self.0.as_object().is_some_and(|fields| fields.values().any(|value| !value.is_null()))
    }
}

impl Drop for Parsed {
    fn drop(&mut self) {
        wipe(self.0.take());
    }
}

fn session_call(account: Option<&str>, method: &'static str, session: u64) -> BusRequest {
    BusRequest::new(account, SDK_API, method, json!({ "session": session }))
}

#[cfg(test)]
mod tests {
    use ramflux_desktop_core::{SecretInput, methods};
    use serde_json::json;

    use super::{Parsed, request};

    #[test]
    fn secret_fields_move_out_of_the_parsed_secret() {
        let mut parsed =
            Parsed(json!({ "token": { "key": "k" }, "slot_id": "s", "extra": "left" }));
        assert_eq!(parsed.take("token"), Some(json!({ "key": "k" })));
        assert_eq!(parsed.text("slot_id").as_deref(), Some("s"));
        assert_eq!(parsed.take("missing"), None);
        assert_eq!(parsed.0["token"], serde_json::Value::Null, "moved out, not copied");
        assert_eq!(parsed.0["slot_id"], serde_json::Value::Null, "moved out, not copied");
        // The remainder ("extra") is wiped when `parsed` drops.
    }

    #[test]
    fn an_event_body_keeps_token_and_event_out_of_the_logged_body() {
        let secret = json!({ "token": { "key": "k" }, "event": { "activate": {} } }).to_string();
        let request = request(
            Some("acct"),
            methods::CARD_SESSION_EVENT,
            &json!({ "surface_id": "s-1", "session": 3 }),
            vec![SecretInput::new(secret)],
        )
        .expect("request");
        assert_eq!(request.body, json!({ "session": 3 }));
        let wire = request.wire_body();
        assert_eq!(wire["token"], json!({ "key": "k" }));
        assert_eq!(wire["event"], json!({ "activate": {} }));
        crate::bus::wipe(wire);
    }
}
