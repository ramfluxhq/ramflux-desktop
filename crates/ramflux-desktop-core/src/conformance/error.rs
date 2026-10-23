// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The renderer error record.
//!
//! A local record shaped like a domain event: what could not be shown, on
//! which surface version, and why. It names ids and kinds only, never block
//! data, so it is safe to log, count and report. [`RendererError::to_effect`]
//! is what a source forwards to the surface's producer once the backend can
//! carry it.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::hash::SurfaceHash;
use super::profile::LimitKind;
use crate::ids::{BlockId, MomentId, SurfaceId};
use crate::intent::{Effect, methods};
use crate::model::BlockKind;

/// Why something could not be rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RendererErrorKind {
    /// The block kind or version is not in the renderer profile.
    UnsupportedBlock,
    /// The block data does not have the shape its kind needs.
    SchemaMismatch,
    /// The surface exceeds a renderer limit; it renders as a whole fallback.
    OverLimit(LimitKind),
    /// The block's privacy forbids displaying it here.
    PrivacyBlocked,
    /// A patch was based on a surface version other than the current one.
    StaleBase,
}

impl RendererErrorKind {
    /// Stable name, for counts and reports.
    pub const fn name(self) -> &'static str {
        match self {
            Self::UnsupportedBlock => "unsupported_block",
            Self::SchemaMismatch => "schema_mismatch",
            Self::OverLimit(_) => "over_limit",
            Self::PrivacyBlocked => "privacy_blocked",
            Self::StaleBase => "stale_base",
        }
    }
}

/// One renderer error.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RendererError {
    /// Moment holding the surface.
    pub moment_id: MomentId,
    /// Surface.
    pub surface_id: SurfaceId,
    /// The surface version it was found on.
    pub surface_hash: SurfaceHash,
    /// The block, for block-level errors.
    pub block_id: Option<BlockId>,
    /// The block kind, for block-level errors.
    pub block_kind: Option<BlockKind>,
    /// Why.
    pub kind: RendererErrorKind,
}

impl RendererError {
    /// The effect that reports this error to the surface's producer.
    pub fn to_effect(&self) -> Effect {
        Effect::Call {
            method: methods::RENDERER_ERROR.to_owned(),
            body: json!({
                "moment_id": self.moment_id,
                "surface_id": self.surface_id,
                "surface_hash": self.surface_hash,
                "block_id": self.block_id,
                "block_kind": self.block_kind,
                "error": self.kind.name(),
                "limit": match self.kind {
                    RendererErrorKind::OverLimit(limit) => json!(limit),
                    _ => serde_json::Value::Null,
                },
            }),
        }
    }
}
