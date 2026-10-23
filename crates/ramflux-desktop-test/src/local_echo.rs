// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Built-in responses every scenario gets for free.
//!
//! - Local echo: local-only settings (pin, unpin, mute, disappearing timer,
//!   default, enabled and removed node profiles, account lock) round-trip in every scenario
//!   without a scripted rule. A scenario rule for the same effect wins.
//! - Audit echo: every call the attended human confirmed with a hold is
//!   recorded in the audit trail, whether or not a scenario rule matched.

use ramflux_desktop_core::{
    AccountId, Actor, AuditEntry, ContextId, Disappearing, DomainEvent, Effect, JsonValue,
    LockState, NodeId, Timestamp, methods,
};

/// Delay of echoed events after the submission, in milliseconds.
pub const ECHO_DELAY_MS: u64 = 50;

/// The local echo for `effect`, or nothing when the effect is not local-only.
pub fn local_echo(effect: &Effect) -> Vec<DomainEvent> {
    let (Some(method), Some(body)) = (effect.method(), effect.body()) else {
        return Vec::new();
    };
    let context = || body.get("context_id").and_then(JsonValue::as_str).map(ContextId::new);
    let node = || body.get("node_id").and_then(JsonValue::as_str).map(NodeId::new);
    let settings = |pinned, muted, disappearing| {
        context().map(|context_id| DomainEvent::ContextSettingsChanged {
            context_id,
            pinned,
            muted,
            disappearing,
        })
    };
    let event = match method {
        methods::CONTEXT_PIN => settings(Some(true), None, None),
        methods::CONTEXT_UNPIN => settings(Some(false), None, None),
        methods::CONTEXT_MUTE => {
            settings(None, body.get("muted").and_then(JsonValue::as_bool), None)
        }
        methods::CONTEXT_DISAPPEARING => {
            let timer = match body.get("seconds").and_then(JsonValue::as_u64) {
                Some(secs) => Disappearing::After(u32::try_from(secs).unwrap_or(u32::MAX)),
                None => Disappearing::Off,
            };
            settings(None, None, Some(timer))
        }
        methods::NODE_SET_DEFAULT => node().map(|node_id| DomainEvent::NodeSettingsChanged {
            node_id,
            enabled: None,
            is_default: Some(true),
        }),
        methods::NODE_SET_ENABLED => node().map(|node_id| DomainEvent::NodeSettingsChanged {
            node_id,
            enabled: body.get("enabled").and_then(JsonValue::as_bool),
            is_default: None,
        }),
        methods::NODE_REMOVE => node().map(|node_id| DomainEvent::NodeRemoved { node_id }),
        methods::ACCOUNT_LOCK => body.get("account_id").and_then(JsonValue::as_str).map(|id| {
            DomainEvent::AccountLockChanged {
                account_id: AccountId::new(id),
                lock_state: LockState::Locked,
            }
        }),
        _ => None,
    };
    event.into_iter().collect()
}

/// The audit record of a call the human confirmed with a hold, if any.
pub fn audit_echo(effect: &Effect, at: Timestamp) -> Option<DomainEvent> {
    let (method, body) = (effect.method()?, effect.body()?);
    if body.get("confirmed_by").is_none_or(JsonValue::is_null) {
        return None;
    }
    let proposed_by = body
        .get("proposed_by")
        .and_then(|value| serde_json::from_value::<Actor>(value.clone()).ok())
        .unwrap_or(Actor::Human);
    let subject =
        ["moment_id", "grant_id", "approval_id", "device_id", "node_id", "participant_id"]
            .into_iter()
            .find_map(|key| body.get(key).and_then(JsonValue::as_str))
            .unwrap_or_default()
            .to_owned();
    Some(DomainEvent::AuditAppended(AuditEntry {
        at,
        action: method.to_owned(),
        subject,
        proposed_by,
        human_confirmed: true,
    }))
}

#[cfg(test)]
mod tests {
    use ramflux_desktop_core::MomentId;
    use serde_json::json;

    use super::*;

    fn call(method: &str, body: JsonValue) -> Effect {
        Effect::Call { method: method.to_owned(), body }
    }

    #[test]
    fn local_settings_echo_back_as_settings_events() {
        let mute = call(methods::CONTEXT_MUTE, json!({ "context_id": "c-1", "muted": true }));
        assert_eq!(
            local_echo(&mute),
            [DomainEvent::ContextSettingsChanged {
                context_id: ContextId::new("c-1"),
                pinned: None,
                muted: Some(true),
                disappearing: None,
            }]
        );
        let off =
            call(methods::CONTEXT_DISAPPEARING, json!({ "context_id": "c-1", "seconds": null }));
        assert!(matches!(
            local_echo(&off).as_slice(),
            [DomainEvent::ContextSettingsChanged { disappearing: Some(Disappearing::Off), .. }]
        ));
        assert!(
            local_echo(&call(methods::SEND_MESSAGE, json!({ "context_id": "c-1" }))).is_empty()
        );
    }

    #[test]
    fn only_confirmed_calls_are_audited() {
        let held = call(
            methods::PRIVACY_REVEAL,
            json!({
                "moment_id": "m-1",
                "proposed_by": { "Agent": "p-orbit" },
                "confirmed_by": { "gesture": "Hold", "held_ms": 1_200 },
            }),
        );
        let Some(DomainEvent::AuditAppended(entry)) = audit_echo(&held, Timestamp(7)) else {
            panic!("expected an audit record");
        };
        assert_eq!(entry.subject, MomentId::new("m-1").as_str());
        assert_eq!(entry.proposed_by, Actor::Agent("p-orbit".into()));
        let plain = call(methods::SEND_MESSAGE, json!({ "confirmed_by": null }));
        assert!(audit_echo(&plain, Timestamp(7)).is_none());
    }
}
