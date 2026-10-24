// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The daemon's answers to surface effects, read by their typed refusal
//! codes (the daemon's `reason_code`, or the code of its error frame; the
//! English message is never read).
//!
//! - `surface.action`: the daemon checks the action against the surface's
//!   current version and hash before it signs and sends it. An action on a
//!   version or content that is no longer current (`stale_version`,
//!   `stale_hash`) is refused: the store hears that the surface changed
//!   after the user reviewed it. Any other refusal is a failed effect with
//!   that refusal's text.
//! - `surface.request_resync`: the request went to the author's devices, or
//!   why not (`rate_limited` is an answer, not an error frame), shown in the
//!   surface.

use ramflux_desktop_core::{
    DomainEvent, EffectFailure, FailureCode, SurfaceEvent, SurfaceId, methods,
};
use ramflux_sdk::SurfaceRefusalCode as Code;
use serde_json::Value;

use super::reason::key;
use super::reason::keyed;
use super::{Translator, text};

/// A typed refusal code, if the daemon's code is one.
pub(super) fn code_of(code: &str) -> Option<Code> {
    serde_json::from_value(Value::String(code.to_owned())).ok()
}

/// The text key of a refusal code (any code the desktop does not know
/// keeps its code).
pub fn refusal_text(code: &str) -> String {
    let Some(known) = code_of(code) else {
        return keyed(key::SURFACE_REFUSED, &[("code", code)]);
    };
    match known {
        Code::StaleVersion => key::SURFACE_STALE_VERSION,
        Code::StaleHash => key::SURFACE_STALE_HASH,
        Code::NotAuthor => key::SURFACE_NOT_AUTHOR,
        Code::NotOutOfSync => key::SURFACE_NOT_OUT_OF_SYNC,
        Code::RateLimited => key::SURFACE_RATE_LIMITED,
        Code::TooLarge => key::SURFACE_TOO_LARGE,
        Code::NotFound => key::SURFACE_NOT_FOUND,
        Code::OutOfSync => key::SURFACE_OUT_OF_SYNC,
        Code::NotActionExecutor => key::SURFACE_NOT_ACTION_EXECUTOR,
        Code::AlreadyUsed => key::SURFACE_ALREADY_USED,
        Code::Closed => key::SURFACE_CLOSED,
        Code::NotPermitted => key::SURFACE_NOT_PERMITTED,
        Code::InvalidRequest => key::SURFACE_INVALID_REQUEST,
    }
    .to_owned()
}

/// The refusal code of a daemon outcome (`accepted: false`), if refused.
/// An outcome without a code is an invalid request.
fn refusal(answer: &Value) -> Option<&str> {
    (answer["accepted"] == Value::Bool(false))
        .then(|| answer["reason_code"].as_str().unwrap_or(Code::InvalidRequest.as_str()))
}

impl Translator {
    /// Events for a performed surface effect.
    pub(super) fn surface_effect_done(
        &mut self,
        method: &str,
        body: &Value,
        answer: &Value,
    ) -> Vec<DomainEvent> {
        let surface_id = SurfaceId::new(text(body, "surface_id"));
        match method {
            methods::SURFACE_ACTION => match refusal(answer) {
                None => Vec::new(),
                Some(code)
                    if matches!(code_of(code), Some(Code::StaleVersion | Code::StaleHash)) =>
                {
                    vec![DomainEvent::Surface(SurfaceEvent::ActionRefused {
                        surface_id,
                        version: body["version"].as_u64().unwrap_or(0),
                    })]
                }
                Some(code) => vec![DomainEvent::EffectFailed(EffectFailure {
                    method: method.to_owned(),
                    code: FailureCode::Daemon(code.to_owned()),
                    message: keyed(
                        key::SURFACE_ACTION_REJECTED,
                        &[("reason", &refusal_text(code))],
                    ),
                })],
            },
            methods::SURFACE_REQUEST_RESYNC => {
                let refused = (answer["rate_limited"] == Value::Bool(true)).then(|| {
                    refusal_text(
                        answer["reason_code"].as_str().unwrap_or(Code::RateLimited.as_str()),
                    )
                });
                vec![DomainEvent::Surface(SurfaceEvent::UpdateRequested { surface_id, refused })]
            }
            _ => Vec::new(),
        }
    }

    /// Events for a surface effect that failed: an update request shows
    /// why in its surface (the daemon's refusal code, or, when the daemon
    /// did not answer, the transport's `message`).
    pub(super) fn surface_effect_failed(
        &self,
        method: &str,
        body: &Value,
        code: &FailureCode,
        message: &str,
    ) -> Vec<DomainEvent> {
        if method != methods::SURFACE_REQUEST_RESYNC {
            return Vec::new();
        }
        let reason = match code {
            FailureCode::Daemon(code) => refusal_text(code),
            FailureCode::Transport | FailureCode::Timeout | FailureCode::UnsupportedByBackend => {
                message.to_owned()
            }
        };
        vec![DomainEvent::Surface(SurfaceEvent::UpdateRequested {
            surface_id: SurfaceId::new(text(body, "surface_id")),
            refused: Some(reason),
        })]
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn refusals_are_read_by_their_code() {
        assert_eq!(refusal(&json!({ "accepted": true })), None);
        assert_eq!(refusal(&json!({ "delivery_state": "submitted" })), None);
        let stale = json!({ "accepted": false, "reason_code": "stale_hash",
            "renderer_error": { "code": "renderer_error", "reject": { "reject": "conflict" } } });
        assert_eq!(refusal(&stale), Some("stale_hash"));
        assert_eq!(refusal(&json!({ "accepted": false })), Some("invalid_request"));
    }

    #[test]
    fn every_daemon_code_has_its_text() {
        for (code, key) in [
            ("stale_version", key::SURFACE_STALE_VERSION),
            ("stale_hash", key::SURFACE_STALE_HASH),
            ("not_author", key::SURFACE_NOT_AUTHOR),
            ("not_out_of_sync", key::SURFACE_NOT_OUT_OF_SYNC),
            ("rate_limited", key::SURFACE_RATE_LIMITED),
            ("too_large", key::SURFACE_TOO_LARGE),
            ("not_found", key::SURFACE_NOT_FOUND),
            ("out_of_sync", key::SURFACE_OUT_OF_SYNC),
            ("not_action_executor", key::SURFACE_NOT_ACTION_EXECUTOR),
            ("already_used", key::SURFACE_ALREADY_USED),
            ("closed", key::SURFACE_CLOSED),
            ("not_permitted", key::SURFACE_NOT_PERMITTED),
            ("invalid_request", key::SURFACE_INVALID_REQUEST),
        ] {
            assert_eq!(code_of(code).map(Code::as_str), Some(code), "the SDK names {code}");
            assert_eq!(refusal_text(code), key);
        }
        assert_eq!(
            refusal_text("ValidationFailed"),
            r#"backend.surface.refused {"code":"ValidationFailed"}"#
        );
    }
}
