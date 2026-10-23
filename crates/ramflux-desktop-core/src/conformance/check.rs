// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A surface checked against a renderer profile.

use std::collections::BTreeMap;

use serde_json::Value;

use super::error::{RendererError, RendererErrorKind};
use super::hash::{SurfaceHash, canonical_len, surface_hash};
use super::profile::{LimitKind, Limits, RendererProfile};
use super::schema::shape_ok;
use crate::ids::{BlockId, MomentId};
use crate::model::{AgentSurfacePayload, BlockKind};

/// Why one block renders as a block fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockIssue {
    /// Kind or version not in the profile.
    Unsupported,
    /// Data shape mismatch.
    Schema,
    /// Display-blocked by its privacy.
    Redacted,
}

impl BlockIssue {
    const fn error_kind(self) -> RendererErrorKind {
        match self {
            Self::Unsupported => RendererErrorKind::UnsupportedBlock,
            Self::Schema => RendererErrorKind::SchemaMismatch,
            Self::Redacted => RendererErrorKind::PrivacyBlocked,
        }
    }
}

/// The result of checking one surface version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceVerdict {
    /// The version checked.
    pub hash: SurfaceHash,
    /// The first limit exceeded; the whole surface then renders as fallback.
    pub over_limit: Option<LimitKind>,
    /// Blocks that render as a block fallback, and why.
    pub blocks: BTreeMap<BlockId, BlockIssue>,
    /// Every renderer error found, in block order.
    pub errors: Vec<RendererError>,
}

impl SurfaceVerdict {
    /// Whether the surface renders (and acts) normally.
    pub fn renders(&self) -> bool {
        self.over_limit.is_none()
    }
}

fn longest_text(value: &Value) -> usize {
    match value {
        Value::String(text) => text.chars().count(),
        Value::Array(items) => items.iter().map(longest_text).max().unwrap_or(0),
        Value::Object(map) => {
            map.iter().map(|(k, v)| k.chars().count().max(longest_text(v))).max().unwrap_or(0)
        }
        _ => 0,
    }
}

/// The first limit `surface` exceeds, if any.
pub fn over_limit(surface: &AgentSurfacePayload, limits: &Limits) -> Option<LimitKind> {
    if surface.blocks.len() > limits.max_blocks {
        return Some(LimitKind::Blocks);
    }
    if surface.actions.len() > limits.max_actions {
        return Some(LimitKind::Actions);
    }
    let labels = surface.actions.iter().flat_map(|a| {
        [a.label.chars().count(), a.disabled_reason.as_deref().map_or(0, |r| r.chars().count())]
    });
    let text = surface
        .blocks
        .iter()
        .map(|b| longest_text(&b.data))
        .chain(labels)
        .chain([surface.fallback_text.chars().count()])
        .max()
        .unwrap_or(0);
    if text > limits.max_text_chars {
        return Some(LimitKind::TextLength);
    }
    let bytes: usize = surface.blocks.iter().map(|b| canonical_len(&b.data)).sum();
    (bytes > limits.max_data_bytes).then_some(LimitKind::DataBytes)
}

/// Checks `surface` (held by `moment`) against `profile`.
pub fn check_surface(
    moment: &MomentId,
    surface: &AgentSurfacePayload,
    profile: &RendererProfile,
) -> SurfaceVerdict {
    let hash = surface_hash(surface);
    let error = |block: Option<(&BlockId, BlockKind)>, kind| RendererError {
        moment_id: moment.clone(),
        surface_id: surface.surface_id.clone(),
        surface_hash: hash.clone(),
        block_id: block.map(|(id, _)| id.clone()),
        block_kind: block.map(|(_, kind)| kind),
        kind,
    };
    if let Some(limit) = over_limit(surface, &profile.limits) {
        let errors = vec![error(None, RendererErrorKind::OverLimit(limit))];
        return SurfaceVerdict { over_limit: Some(limit), blocks: BTreeMap::new(), errors, hash };
    }
    let mut blocks = BTreeMap::new();
    let mut errors = Vec::new();
    for block in &surface.blocks {
        let issue = if block.redacted {
            Some(BlockIssue::Redacted)
        } else if !profile.supports(block.kind, block.version) {
            Some(BlockIssue::Unsupported)
        } else if !shape_ok(block) {
            Some(BlockIssue::Schema)
        } else {
            None
        };
        if let Some(issue) = issue {
            errors.push(error(Some((&block.id, block.kind)), issue.error_kind()));
            blocks.insert(block.id.clone(), issue);
        }
    }
    SurfaceVerdict { hash, over_limit: None, blocks, errors }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::conformance::{BlockSupport, DeviceClass};
    use crate::ids::SurfaceId;
    use crate::model::{Block, SurfaceAction};

    fn profile() -> RendererProfile {
        RendererProfile {
            device_class: DeviceClass::Desktop,
            runtime: "test".to_owned(),
            blocks: vec![BlockSupport { kind: BlockKind::Text, versions: vec![1] }],
            input: Vec::new(),
            output: Vec::new(),
            surface_modes: Vec::new(),
            limits: Limits {
                max_blocks: 4,
                max_actions: 1,
                max_text_chars: 10,
                max_data_bytes: 60,
            },
        }
    }

    fn surface(blocks: Vec<Block>, actions: usize) -> AgentSurfacePayload {
        AgentSurfacePayload {
            surface_id: SurfaceId::new("s"),
            fallback_text: "f".to_owned(),
            blocks,
            actions: (0..actions)
                .map(|i| SurfaceAction {
                    id: format!("a{i}"),
                    label: "A".to_owned(),
                    risk: crate::model::Risk::Low,
                    disabled_reason: None,
                })
                .collect(),
            streaming: false,
        }
    }

    fn text(id: &str, text: &str) -> Block {
        Block::new(id, BlockKind::Text, json!({ "text": text }))
    }

    #[test]
    fn each_limit_is_reported_and_blocks_are_not_checked_beyond_it() {
        let limits = profile().limits;
        let many = (0..5).map(|i| text(&format!("b{i}"), "x")).collect();
        assert_eq!(over_limit(&surface(many, 0), &limits), Some(LimitKind::Blocks));
        assert_eq!(over_limit(&surface(vec![], 2), &limits), Some(LimitKind::Actions));
        let long = surface(vec![text("b", "0123456789x")], 0);
        assert_eq!(over_limit(&long, &limits), Some(LimitKind::TextLength));
        let bulky = surface((0..3).map(|i| text(&format!("b{i}"), "0123456789")).collect(), 0);
        assert_eq!(over_limit(&bulky, &limits), Some(LimitKind::DataBytes));
        let verdict = check_surface(&MomentId::new("m"), &long, &profile());
        assert!(!verdict.renders());
        assert!(verdict.blocks.is_empty());
        assert_eq!(verdict.errors.len(), 1);
    }

    #[test]
    fn block_issues_are_privacy_then_support_then_shape() {
        let blocks = vec![
            text("ok", "fine"),
            Block { redacted: true, ..Block::new("hidden", BlockKind::Markdown, json!({})) },
            Block::new("chart", BlockKind::Unsupported, json!({})),
            Block::new("bad", BlockKind::Text, json!({ "text": 1 })),
        ];
        let verdict = check_surface(&MomentId::new("m"), &surface(blocks, 0), &profile());
        assert!(verdict.renders());
        let issues: Vec<(&str, BlockIssue)> =
            verdict.blocks.iter().map(|(id, issue)| (id.as_str(), *issue)).collect();
        assert_eq!(
            issues,
            [
                ("bad", BlockIssue::Schema),
                ("chart", BlockIssue::Unsupported),
                ("hidden", BlockIssue::Redacted),
            ]
        );
        assert_eq!(verdict.errors.len(), 3);
        assert!(verdict.errors.iter().all(|e| e.surface_hash == verdict.hash));
    }
}
