// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Versioned agent surface events, as a conversation carries them.
//!
//! An agent surface is a conversation message: its author creates it
//! (version 1), then patches it against the content hash of the version it
//! last saw, streams text into its text blocks, and closes it. Every
//! mutation consumes exactly the next version. The surface is declarative
//! data only (blocks and actions), never code, and always carries a
//! fallback text that clients without a renderer show instead.
//!
//! The store applies these events with [`crate::Store::apply`]; a patch
//! whose base hash is not the current content leaves the surface in
//! conflict, and an event that would take the surface over a renderer
//! limit leaves it showing only its fallback text.

use serde::{Deserialize, Serialize};

use crate::conformance::SurfaceHash;
use crate::ids::{BlockId, ContextId, MomentId, ParticipantId, SurfaceId};
use crate::model::{Block, SurfaceAction};

/// One operation of a surface patch: a small, closed set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SurfacePatchOp {
    /// Replace the block with the same id.
    ReplaceBlock {
        /// The new block.
        block: Block,
    },
    /// Insert a block at `index` (`0..=len`).
    InsertBlock {
        /// Position.
        index: usize,
        /// The new block; its id must be new to the surface.
        block: Block,
    },
    /// Remove a block.
    RemoveBlock {
        /// The block.
        block_id: BlockId,
    },
    /// Replace every action.
    SetActions {
        /// The new actions.
        actions: Vec<SurfaceAction>,
    },
}

/// A versioned event on one agent surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SurfaceEvent {
    /// The author posts a new surface into a conversation (version 1).
    Create {
        /// The conversation message presenting the surface.
        moment_id: MomentId,
        /// Conversation.
        context_id: ContextId,
        /// Author.
        origin: ParticipantId,
        /// Surface id, new to the store.
        surface_id: SurfaceId,
        /// Must be 1.
        version: u64,
        /// Ordered blocks.
        #[serde(default)]
        blocks: Vec<Block>,
        /// Offered actions.
        #[serde(default)]
        actions: Vec<SurfaceAction>,
        /// Text shown where the surface cannot be rendered. Required.
        fallback_text: String,
    },
    /// The author changes the surface it last saw as `base_hash`.
    Patch {
        /// Surface.
        surface_id: SurfaceId,
        /// The next version.
        version: u64,
        /// Content hash of the version the patch was made against.
        base_hash: SurfaceHash,
        /// Operations, applied in order, all or nothing.
        ops: Vec<SurfacePatchOp>,
    },
    /// The author appends text to an open text, markdown or log block.
    StreamAppend {
        /// Surface.
        surface_id: SurfaceId,
        /// The next version.
        version: u64,
        /// Target block.
        block_id: BlockId,
        /// Text to append (at most [`crate::conformance::Limits::max_text_delta_bytes`]).
        text_delta: String,
        /// The block's stream ends with this append.
        #[serde(rename = "final", default)]
        is_final: bool,
    },
    /// The author closes the surface: it stays as a frozen record.
    Close {
        /// Surface.
        surface_id: SurfaceId,
        /// The next version.
        version: u64,
    },
}

impl SurfaceEvent {
    /// The surface the event is about.
    pub fn surface_id(&self) -> &SurfaceId {
        match self {
            Self::Create { surface_id, .. }
            | Self::Patch { surface_id, .. }
            | Self::StreamAppend { surface_id, .. }
            | Self::Close { surface_id, .. } => surface_id,
        }
    }

    /// The version the event produces.
    pub const fn version(&self) -> u64 {
        match self {
            Self::Create { version, .. }
            | Self::Patch { version, .. }
            | Self::StreamAppend { version, .. }
            | Self::Close { version, .. } => *version,
        }
    }
}
