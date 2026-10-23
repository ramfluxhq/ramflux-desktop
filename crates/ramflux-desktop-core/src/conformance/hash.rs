// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The canonical content hash of a surface version.
//!
//! The hash covers everything the user reviews (surface id, fallback text,
//! blocks with their data, actions with labels, risks and disabled reasons)
//! and nothing else: the streaming flag is not content. The encoding is
//! canonical JSON (object keys sorted, no whitespace) hashed with SHA-256
//! under a domain tag, so equal content gives an equal hash whatever order
//! a peer wrote its keys in. Surface content is peer-controlled, so the
//! hash must resist collisions; a plain checksum would let a peer change an
//! action under a pending confirmation without changing the hash.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::model::AgentSurfacePayload;

/// Domain tag hashed before the canonical encoding.
const DOMAIN: &[u8] = b"ramflux.desktop.surface.v1\n";

/// Content hash of one surface version, `sha256:<hex>`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SurfaceHash(String);

impl SurfaceHash {
    /// Wraps a hash received from elsewhere.
    pub fn new(hash: impl Into<String>) -> Self {
        Self(hash.into())
    }

    /// The `sha256:<hex>` text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A short form for display (first 12 hex digits).
    pub fn short(&self) -> &str {
        let hex = self.0.strip_prefix("sha256:").unwrap_or(&self.0);
        hex.get(..12).unwrap_or(hex)
    }
}

impl fmt::Debug for SurfaceHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SurfaceHash({})", self.short())
    }
}

impl fmt::Display for SurfaceHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The content hash of `surface`.
pub fn surface_hash(surface: &AgentSurfacePayload) -> SurfaceHash {
    let content = json!({
        "surface_id": surface.surface_id,
        "fallback_text": surface.fallback_text,
        "blocks": surface.blocks,
        "actions": surface.actions,
    });
    let mut text = String::new();
    canonical(&content, &mut text);
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(7 + digest.len() * 2);
    hex.push_str("sha256:");
    for byte in digest {
        hex.push(char::from(HEX[usize::from(byte >> 4)]));
        hex.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    SurfaceHash(hex)
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// Writes `value` as canonical JSON: sorted keys, no whitespace.
pub(crate) fn canonical(value: &Value, out: &mut String) {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => out.push_str(&value.to_string()),
        Value::String(text) => out.push_str(&Value::String(text.clone()).to_string()),
        Value::Array(items) => {
            out.push('[');
            for (ix, item) in items.iter().enumerate() {
                if ix > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (ix, key) in keys.into_iter().enumerate() {
                if ix > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                canonical(&map[key], out);
            }
            out.push('}');
        }
    }
}

/// Bytes of the canonical encoding of `value`.
pub(crate) fn canonical_len(value: &Value) -> usize {
    let mut text = String::new();
    canonical(value, &mut text);
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::SurfaceId;
    use crate::model::{Block, BlockKind, SurfaceAction};

    fn surface(data: Value) -> AgentSurfacePayload {
        AgentSurfacePayload {
            surface_id: SurfaceId::new("s-1"),
            fallback_text: "fallback".to_owned(),
            blocks: vec![Block::new("b-1", BlockKind::Text, data)],
            actions: vec![SurfaceAction {
                id: "go".to_owned(),
                label: "Go".to_owned(),
                risk: crate::model::Risk::High,
                disabled_reason: None,
            }],
            streaming: false,
        }
    }

    #[test]
    fn key_order_does_not_change_the_hash() {
        let a: Value = serde_json::from_str(r#"{"text":"hi","extra":{"b":1,"a":2}}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"extra":{"a":2,"b":1},"text":"hi"}"#).unwrap();
        assert_eq!(surface_hash(&surface(a)), surface_hash(&surface(b)));
    }

    #[test]
    fn content_changes_the_hash_and_streaming_does_not() {
        let base = surface(json!({ "text": "hi" }));
        let hash = surface_hash(&base);
        assert!(hash.as_str().starts_with("sha256:") && hash.as_str().len() == 7 + 64);
        let mut text = base.clone();
        text.blocks[0].data = json!({ "text": "hi!" });
        assert_ne!(surface_hash(&text), hash);
        let mut label = base.clone();
        label.actions[0].label = "Delete everything".to_owned();
        assert_ne!(surface_hash(&label), hash);
        let mut risk = base.clone();
        risk.actions[0].risk = crate::model::Risk::Low;
        assert_ne!(surface_hash(&risk), hash);
        let mut streaming = base.clone();
        streaming.streaming = true;
        assert_eq!(surface_hash(&streaming), hash);
    }

    #[test]
    fn canonical_json_escapes_strings_and_sorts_keys() {
        let mut out = String::new();
        canonical(&json!({ "b": "q\"x", "a": [1, null, true] }), &mut out);
        assert_eq!(out, r#"{"a":[1,null,true],"b":"q\"x"}"#);
    }
}
