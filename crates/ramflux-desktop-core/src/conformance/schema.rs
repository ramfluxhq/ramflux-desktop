// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The data shape each block kind needs (version 1).
//!
//! Block data is untrusted. A block whose data lacks what its kind needs is
//! a schema mismatch: it renders as a block fallback, never half-drawn.

use serde_json::Value;

use crate::model::{Block, BlockKind};

fn string(data: &Value, key: &str) -> bool {
    data.get(key).is_some_and(Value::is_string)
}

fn array<'a>(data: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
    data.get(key).and_then(Value::as_array)
}

fn key_value(data: &Value) -> bool {
    array(data, "items")
        .is_some_and(|items| items.iter().all(|p| p.as_array().is_some_and(|p| p.len() == 2)))
}

fn table(data: &Value) -> bool {
    let Some(width) = array(data, "columns").map(Vec::len).filter(|w| *w > 0) else {
        return false;
    };
    array(data, "rows")
        .is_some_and(|rows| rows.iter().all(|r| r.as_array().is_some_and(|r| r.len() == width)))
}

fn select(data: &Value) -> bool {
    string(data, "label")
        && string(data, "name")
        && array(data, "options").is_some_and(|options| {
            options.iter().all(|option| match option {
                Value::String(_) => true,
                Value::Array(pair) => pair.first().is_some_and(Value::is_string),
                _ => false,
            })
        })
}

fn action_group(data: &Value) -> bool {
    array(data, "actions").is_some_and(|ids| !ids.is_empty() && ids.iter().all(Value::is_string))
}

/// Whether `block`'s data has the shape its kind needs.
///
/// `Fallback` blocks need nothing (they show the surface's fallback text);
/// `Unsupported` blocks have no shape to check.
pub fn shape_ok(block: &Block) -> bool {
    let data = &block.data;
    match block.kind {
        BlockKind::Text
        | BlockKind::Markdown
        | BlockKind::Heading
        | BlockKind::LogStream
        | BlockKind::Warning
        | BlockKind::Error
        | BlockKind::Success => string(data, "text"),
        BlockKind::KeyValue => key_value(data),
        BlockKind::Metric => string(data, "label") && string(data, "value"),
        BlockKind::List => array(data, "items").is_some(),
        BlockKind::TableFallback => table(data),
        BlockKind::Form => string(data, "title"),
        BlockKind::Input => string(data, "label") && string(data, "name"),
        BlockKind::Select => select(data),
        BlockKind::Progress => data.get("value").is_some_and(Value::is_number),
        BlockKind::Approval => string(data, "grant_id") && string(data, "summary"),
        BlockKind::ActionGroup => action_group(data),
        BlockKind::FilePreview => string(data, "name"),
        BlockKind::Fallback | BlockKind::Unsupported => true,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn shapes_follow_the_catalog() {
        let ok = |kind, data| shape_ok(&Block::new("b", kind, data));
        assert!(ok(BlockKind::Text, json!({ "text": "x" })));
        assert!(!ok(BlockKind::Text, json!({ "text": 3 })));
        assert!(ok(BlockKind::TableFallback, json!({ "columns": ["a"], "rows": [["1"]] })));
        assert!(!ok(BlockKind::TableFallback, json!({ "columns": ["a"], "rows": [["1", "2"]] })));
        assert!(!ok(BlockKind::KeyValue, json!({ "items": [["k"]] })));
        assert!(ok(
            BlockKind::Select,
            json!({ "label": "L", "name": "n", "options": ["a", ["b", "B"]] })
        ));
        assert!(!ok(BlockKind::ActionGroup, json!({ "actions": [] })));
        assert!(!ok(BlockKind::Approval, json!({ "grant_id": "g" })));
        assert!(ok(BlockKind::Fallback, Value::Null));
    }
}
