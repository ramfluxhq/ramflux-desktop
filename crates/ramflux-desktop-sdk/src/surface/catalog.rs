// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Surface snapshots (the protocol's `createSurface` content) as desktop
//! blocks and actions, and back.
//!
//! A protocol component is a JSON object with an `id` and a component type
//! (`type`, or `kind`), its data either nested under `data` or written
//! beside the id (the agent host writes `{"id":"reply","type":"Text",
//! "text":"…"}`); the protocol streams text into `text` or `data.text`. A
//! type outside the desktop catalog becomes an unsupported block, which the
//! renderer shows as the surface's fallback text. An action is an object
//! with `name` (or `id`), `label`, and optionally `risk` and
//! `disabled_reason`, or a bare action id.
//!
//! The way back ([`component_of`], [`command_of`]) writes the nested form;
//! it exists to author fixtures and replay scenarios, never to re-encode a
//! peer's surface (the daemon validates peers' original bytes).

use ramflux_desktop_core::{AgentSurfacePayload, Block, BlockKind, Risk, SurfaceAction, SurfaceId};
use ramflux_protocol::{CreateSurfaceCommand, SurfaceType};
use serde_json::{Map, Value, json};

/// The catalog the desktop writes when it authors a surface (fixtures).
pub const CATALOG: &str = "basic";
/// Its version.
pub const CATALOG_VERSION: &str = "1";

/// Fields of a flat component that are not block data.
const COMPONENT_FIELDS: [&str; 6] = ["id", "type", "kind", "version", "redacted", "component"];

/// Desktop block kinds by their catalog names.
const KINDS: [(&str, BlockKind); 19] = [
    ("Text", BlockKind::Text),
    ("Markdown", BlockKind::Markdown),
    ("Heading", BlockKind::Heading),
    ("KeyValue", BlockKind::KeyValue),
    ("Metric", BlockKind::Metric),
    ("List", BlockKind::List),
    ("TableFallback", BlockKind::TableFallback),
    ("Form", BlockKind::Form),
    ("Input", BlockKind::Input),
    ("Select", BlockKind::Select),
    ("Progress", BlockKind::Progress),
    ("LogStream", BlockKind::LogStream),
    ("Warning", BlockKind::Warning),
    ("Error", BlockKind::Error),
    ("Success", BlockKind::Success),
    ("Approval", BlockKind::Approval),
    ("ActionGroup", BlockKind::ActionGroup),
    ("FilePreview", BlockKind::FilePreview),
    ("Fallback", BlockKind::Fallback),
];

/// The kind a component type names: `Text`, `text`, `key_value` and
/// `keyValue` all name one kind; anything else is unsupported.
pub fn kind_of(name: &str) -> BlockKind {
    let folded: String = name.chars().filter(|c| *c != '_' && *c != '-').collect();
    KINDS
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(&folded))
        .map_or(BlockKind::Unsupported, |(_, kind)| *kind)
}

/// The catalog name of a kind.
pub fn kind_name(kind: BlockKind) -> &'static str {
    KINDS.iter().find(|(_, k)| *k == kind).map_or("Unsupported", |(name, _)| name)
}

/// One component as a block, or `None` when it has no id (the protocol
/// never accepts one; a snapshot read from the daemon always has them).
pub fn block_of(component: &Value) -> Option<Block> {
    let object = component.as_object()?;
    let id = object.get("id").and_then(Value::as_str).filter(|id| !id.is_empty())?;
    let kind = ["type", "kind", "component"]
        .iter()
        .find_map(|field| object.get(*field).and_then(Value::as_str))
        .map_or(BlockKind::Unsupported, kind_of);
    let data = match object.get("data") {
        Some(data) => data.clone(),
        None => Value::Object(
            object
                .iter()
                .filter(|(key, _)| !COMPONENT_FIELDS.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
    };
    let version = object
        .get("version")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(ramflux_desktop_core::DEFAULT_BLOCK_VERSION);
    let redacted = object.get("redacted").and_then(Value::as_bool).unwrap_or(false);
    Some(Block { id: id.into(), kind, version, redacted, data })
}

fn risk_of(value: Option<&Value>) -> Risk {
    match value.and_then(Value::as_str).map(str::to_ascii_lowercase).as_deref() {
        Some("medium") => Risk::Medium,
        Some("high") => Risk::High,
        Some("critical") => Risk::Critical,
        _ => Risk::Low,
    }
}

/// One allowed action as a surface action, or `None` without an id.
pub fn action_of(value: &Value) -> Option<SurfaceAction> {
    if let Some(id) = value.as_str().filter(|id| !id.is_empty()) {
        return Some(SurfaceAction {
            id: id.to_owned(),
            label: id.to_owned(),
            risk: Risk::Low,
            disabled_reason: None,
        });
    }
    let object = value.as_object()?;
    let id = ["name", "id"]
        .iter()
        .find_map(|field| object.get(*field).and_then(Value::as_str))
        .filter(|id| !id.is_empty())?;
    let label = object.get("label").and_then(Value::as_str).unwrap_or(id);
    let disabled = object
        .get("disabled_reason")
        .or_else(|| object.get("disabledReason"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            (object.get("disabled").and_then(Value::as_bool) == Some(true))
                .then(|| "Unavailable".to_owned())
        });
    Some(SurfaceAction {
        id: id.to_owned(),
        label: label.to_owned(),
        risk: risk_of(object.get("risk")),
        disabled_reason: disabled,
    })
}

/// A snapshot as the desktop payload of surface `surface` (the scoped id
/// the desktop keys it by). Components without an id are dropped.
pub fn payload_of(snapshot: &Value, surface: SurfaceId) -> AgentSurfacePayload {
    let list =
        |field: &str| snapshot.get(field).and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    AgentSurfacePayload {
        surface_id: surface,
        fallback_text: snapshot
            .get("fallbackText")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        blocks: list("components").iter().filter_map(block_of).collect(),
        actions: list("allowedActions").iter().filter_map(action_of).collect(),
        streaming: false,
    }
}

/// A block as a component, in the nested form.
pub fn component_of(block: &Block) -> Value {
    let mut component = Map::new();
    component.insert("id".into(), json!(block.id));
    component.insert("type".into(), json!(kind_name(block.kind)));
    if block.version != ramflux_desktop_core::DEFAULT_BLOCK_VERSION {
        component.insert("version".into(), json!(block.version));
    }
    if block.redacted {
        component.insert("redacted".into(), json!(true));
    }
    component.insert("data".into(), block.data.clone());
    Value::Object(component)
}

/// A surface action as an allowed action.
pub fn allowed_action_of(action: &SurfaceAction) -> Value {
    let mut value = json!({ "name": action.id, "label": action.label });
    if action.risk != Risk::Low {
        value["risk"] = json!(format!("{:?}", action.risk).to_ascii_lowercase());
    }
    if let Some(reason) = &action.disabled_reason {
        value["disabled_reason"] = json!(reason);
    }
    value
}

/// A desktop surface as the protocol's `createSurface` content with the
/// protocol surface id `surface_id` (fixtures and replays).
pub fn command_of(surface_id: &str, payload: &AgentSurfacePayload) -> CreateSurfaceCommand {
    CreateSurfaceCommand {
        surface_id: surface_id.to_owned(),
        surface_type: SurfaceType::MessageCard,
        catalog: CATALOG.to_owned(),
        catalog_version: CATALOG_VERSION.to_owned(),
        fallback_text: payload.fallback_text.clone(),
        fallback_markdown: None,
        components: payload.blocks.iter().map(component_of).collect(),
        data_model: Map::new(),
        allowed_actions: payload.actions.iter().map(allowed_action_of).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_hosts_flat_components_and_named_actions_are_read() {
        let snapshot = json!({
            "surfaceId": "s", "surfaceType": "status_panel", "catalog": "basic",
            "catalogVersion": "1", "fallbackText": "Pick a day",
            "components": [
                { "id": "reply", "type": "Text", "text": "Hello" },
                { "id": "chart", "type": "Chart", "data": { "points": [1] } },
                { "type": "Text", "text": "no id" },
                { "id": "kv", "kind": "key_value", "data": { "items": [["a", "b"]] } },
            ],
            "allowedActions": [
                { "name": "thu", "label": "Thursday" },
                { "id": "fri", "label": "Friday", "risk": "high", "disabled_reason": "QA" },
                "mon",
            ],
        });
        let payload = payload_of(&snapshot, SurfaceId::new("ref"));
        assert_eq!(payload.fallback_text, "Pick a day");
        let blocks: Vec<(&str, BlockKind)> =
            payload.blocks.iter().map(|b| (b.id.as_str(), b.kind)).collect();
        assert_eq!(
            blocks,
            [
                ("reply", BlockKind::Text),
                ("chart", BlockKind::Unsupported),
                ("kv", BlockKind::KeyValue)
            ]
        );
        assert_eq!(payload.blocks[0].data, json!({ "text": "Hello" }));
        assert_eq!(payload.actions[0].label, "Thursday");
        assert_eq!(payload.actions[1].risk, Risk::High);
        assert_eq!(payload.actions[1].disabled_reason.as_deref(), Some("QA"));
        assert_eq!(payload.actions[2].id, "mon");
    }

    #[test]
    fn a_desktop_surface_survives_the_round_trip() {
        let payload = AgentSurfacePayload {
            surface_id: SurfaceId::new("ref"),
            fallback_text: "Status".into(),
            blocks: vec![
                Block::new("h", BlockKind::Heading, json!({ "text": "Launch" })),
                Block {
                    version: 2,
                    ..Block::new("p", BlockKind::Progress, json!({ "value": 0.5 }))
                },
            ],
            actions: vec![SurfaceAction {
                id: "go".into(),
                label: "Go".into(),
                risk: Risk::Critical,
                disabled_reason: None,
            }],
            streaming: false,
        };
        let command = command_of("s-status", &payload);
        let snapshot = serde_json::to_value(&command).expect("json");
        assert_eq!(payload_of(&snapshot, SurfaceId::new("ref")), payload);
        assert!(ramflux_protocol::surface_snapshot_hash(&command).is_ok());
    }
}
