// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A surface action as the protocol's action bytes, bound to the version
//! and hash the user saw.
//!
//! The bytes are canonical JSON of the protocol's v2 action (the arguments
//! hashed with the protocol's own argument hash), checked by the protocol's
//! only ingress constructor before they leave. An action that answers a
//! task step carries `taskBinding {task_id, step_id}` inside the signed
//! bytes, so the step can be resolved from the action (or the author's
//! receipt, which embeds it) alone, even after a restart. The daemon signs, dedupes
//! and delivers them to the author; it refuses an action on a version that
//! is no longer current (`stale`), which the desktop shows as "changed
//! after you reviewed it".

use ramflux_protocol::{A2UI_ACTION_SCHEMA, A2uiSurfaceAction};
use serde_json::{Value, json};

use super::SurfaceTarget;

/// What the user acted on.
#[derive(Debug, Clone)]
pub struct Shown<'a> {
    /// The version the action was shown on.
    pub version: u64,
    /// Its hash, as the daemon reported it.
    pub surface_hash: &'a str,
    /// The action.
    pub action_id: &'a str,
    /// Values the surface collected (`null` when none).
    pub values: &'a Value,
    /// The task step the action answers (the daemon's task id, the step
    /// id), signed as the action's `taskBinding`.
    pub task_binding: Option<(&'a str, &'a str)>,
}

/// The canonical action bytes.
///
/// # Errors
///
/// Returns a message when the values are not I-JSON or the protocol
/// refuses the action's shape.
pub fn action_bytes(target: &SurfaceTarget, shown: &Shown<'_>) -> Result<Vec<u8>, String> {
    let values = if shown.values.is_null() { json!({}) } else { shown.values.clone() };
    let args_hash = ramflux_protocol::surface_args_hash(&values).map_err(|e| e.to_string())?;
    let mut action = json!({
        "schema": A2UI_ACTION_SCHEMA,
        "conversationId": target.conversation_id,
        "surfaceId": target.surface_id,
        "version": shown.version,
        "surfaceHash": shown.surface_hash,
        "actionId": shown.action_id,
        "values": values,
        "argsHash": args_hash,
    });
    if let Some((task_id, step_id)) = shown.task_binding {
        action["taskBinding"] = json!({ "task_id": task_id, "step_id": step_id });
    }
    let bytes = ramflux_protocol::canonical_json_bytes(&action).map_err(|e| e.to_string())?;
    A2uiSurfaceAction::from_wire_bytes(&bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// The `surface.action` request body: the bytes, a fresh logical message
/// id, and the author the action goes back to.
///
/// # Errors
///
/// Returns a message when the bytes cannot be built or the platform has no
/// randomness for the message id.
pub fn action_body(target: &SurfaceTarget, shown: &Shown<'_>) -> Result<Value, String> {
    let bytes = action_bytes(target, shown)?;
    let request = ramflux_sdk::SurfaceSendRequest::action(&target.author_principal_id, &bytes)
        .map_err(|e| e.to_string())?;
    serde_json::to_value(request).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> SurfaceTarget {
        SurfaceTarget {
            conversation_id: "thread-team".into(),
            author_principal_id: "principal-bo".into(),
            surface_id: "s-pick".into(),
        }
    }

    #[test]
    fn the_action_binds_the_shown_version_and_hashes_its_arguments() {
        let values = json!({ "note": "x" });
        let shown = Shown {
            version: 3,
            surface_hash: "Qh3",
            action_id: "thu",
            values: &values,
            task_binding: None,
        };
        let body = action_body(&target(), &shown).expect("body");
        assert_eq!(body["conversation_id"], "thread-team");
        assert_eq!(body["author_principal_id"], "principal-bo");
        let wire = ramflux_protocol::decode_base64url(body["wire_base64"].as_str().expect("wire"))
            .expect("base64url");
        let action = A2uiSurfaceAction::from_wire_bytes(&wire).expect("protocol action");
        assert_eq!(
            (action.surface_id(), action.version(), action.surface_hash()),
            ("s-pick", 3, "Qh3")
        );
        assert_eq!(action.action_id(), "thu");
        assert_eq!(action.args_hash(), ramflux_protocol::surface_args_hash(&values).expect("hash"));
        let again = action_body(&target(), &shown).expect("body");
        assert_ne!(again["message_id"], body["message_id"], "every action is its own message");
    }

    #[test]
    fn a_button_without_values_sends_an_empty_object() {
        let shown = Shown {
            version: 1,
            surface_hash: "h",
            action_id: "ok",
            values: &Value::Null,
            task_binding: None,
        };
        let wire = action_bytes(&target(), &shown).expect("bytes");
        let action = A2uiSurfaceAction::from_wire_bytes(&wire).expect("action");
        assert_eq!(action.values(), &json!({}));
        assert!(action.task_binding().is_none());
    }

    #[test]
    fn an_answer_to_a_task_step_signs_the_step_it_answers() {
        let values = json!({ "decision": "resume" });
        let shown = Shown {
            version: 2,
            surface_hash: "h",
            action_id: "thu",
            values: &values,
            task_binding: Some(("t-day", "pick")),
        };
        let wire = action_bytes(&target(), &shown).expect("bytes");
        let action = A2uiSurfaceAction::from_wire_bytes(&wire).expect("action");
        let binding = action.task_binding().expect("bound");
        assert_eq!((binding.task_id.as_str(), binding.step_id.as_str()), ("t-day", "pick"));
    }
}
