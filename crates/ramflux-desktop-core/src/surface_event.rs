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
//! Each version carries its content hash as the source reports it (the
//! surface protocol's snapshot hash); a source without one leaves it out and
//! the store labels the version instead ([`SurfaceHash::of_version`]).
//!
//! A source that reads the surface's current state from an authority (the
//! daemon) rather than receiving each event sends a [`SurfaceSnapshot`]:
//! the whole state at some version, which may skip versions the source did
//! not see one by one, and says whether the authority knows it is out of
//! sync with the author.
//!
//! The store applies these events with [`crate::Store::apply`]; a patch
//! whose base hash is not the current content leaves the surface in
//! conflict, and an event that would take the surface over a renderer
//! limit leaves it showing only its fallback text.

use serde::{Deserialize, Serialize};

use crate::conformance::SurfaceHash;
use crate::ids::{BlockId, ContextId, MomentId, ParticipantId, SurfaceId, Timestamp};
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
    /// Replace the fallback text (it stays true after the content changed).
    SetFallback {
        /// The new fallback text; required, never empty.
        fallback_text: String,
    },
}

/// The whole current state of a surface, as an authority holds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceSnapshot {
    /// The conversation message presenting the surface.
    pub moment_id: MomentId,
    /// Conversation.
    pub context_id: ContextId,
    /// Author.
    pub origin: ParticipantId,
    /// When the surface was posted, if the source knows (else the time the
    /// source saw it).
    #[serde(default)]
    pub at: Option<Timestamp>,
    /// Surface id.
    pub surface_id: SurfaceId,
    /// The version this state is (at least 1).
    pub version: u64,
    /// Its content hash.
    pub hash: SurfaceHash,
    /// Ordered blocks.
    #[serde(default)]
    pub blocks: Vec<Block>,
    /// Offered actions.
    #[serde(default)]
    pub actions: Vec<SurfaceAction>,
    /// Text shown where the surface cannot be rendered. Required.
    pub fallback_text: String,
    /// The author closed it.
    #[serde(default)]
    pub closed: bool,
    /// The authority knows it missed a change of the author's: this state
    /// is not the author's current one, and actions on it are refused until
    /// it catches up.
    #[serde(default)]
    pub out_of_sync: bool,
    /// Blocks whose text the author is still streaming, as far as the
    /// source can tell.
    #[serde(default)]
    pub open_streams: Vec<BlockId>,
    /// Blocks whose stream the authority reports final (no more appends):
    /// their streams end here, whatever was seen of them before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub final_streams: Vec<BlockId>,
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
        /// Content hash of version 1, when the source has one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hash: Option<SurfaceHash>,
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
        /// Content hash of the resulting version, when the source has one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hash: Option<SurfaceHash>,
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
        /// Content hash of the resulting version, when the source has one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hash: Option<SurfaceHash>,
    },
    /// The author closes the surface: it stays as a frozen record.
    Close {
        /// Surface.
        surface_id: SurfaceId,
        /// The next version.
        version: u64,
    },
    /// The authority's current state of a surface (created when new).
    Snapshot(Box<SurfaceSnapshot>),
    /// The authority refused an action the user took on `version`: the
    /// surface had changed. Nothing was done; the user reviews it again.
    ActionRefused {
        /// Surface.
        surface_id: SurfaceId,
        /// The version the action was shown on.
        version: u64,
    },
    /// The answer to the user's request for the author's current state of
    /// an out-of-sync surface: sent, or refused with the reason.
    UpdateRequested {
        /// Surface.
        surface_id: SurfaceId,
        /// Why the request could not be sent, when it could not.
        #[serde(default)]
        refused: Option<String>,
    },
    /// The authority's last-updated time of a surface (the author's clock,
    /// display only); set after the versioned event that brought it.
    UpdatedAt {
        /// Surface.
        surface_id: SurfaceId,
        /// When the author last changed it.
        at: Timestamp,
    },
    /// The authority no longer holds the surface: its moment is removed.
    Removed {
        /// Surface.
        surface_id: SurfaceId,
    },
}

impl SurfaceEvent {
    /// The surface the event is about.
    pub fn surface_id(&self) -> &SurfaceId {
        match self {
            Self::Create { surface_id, .. }
            | Self::Patch { surface_id, .. }
            | Self::StreamAppend { surface_id, .. }
            | Self::Close { surface_id, .. }
            | Self::ActionRefused { surface_id, .. }
            | Self::UpdateRequested { surface_id, .. }
            | Self::UpdatedAt { surface_id, .. }
            | Self::Removed { surface_id } => surface_id,
            Self::Snapshot(snapshot) => &snapshot.surface_id,
        }
    }

    /// The version the event produces (or names).
    pub fn version(&self) -> u64 {
        match self {
            Self::Create { version, .. }
            | Self::Patch { version, .. }
            | Self::StreamAppend { version, .. }
            | Self::Close { version, .. }
            | Self::ActionRefused { version, .. } => *version,
            Self::Snapshot(snapshot) => snapshot.version,
            Self::UpdateRequested { .. } | Self::UpdatedAt { .. } | Self::Removed { .. } => 0,
        }
    }
}
