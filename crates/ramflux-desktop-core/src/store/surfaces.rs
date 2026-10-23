// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Agent surface versions: what the store knows about every surface beside
//! its content (version, content hash, state, open text streams), and the
//! unversioned paths that change a surface (a full copy in a moment
//! payload, [`DomainEvent::SurfacePatched`](crate::DomainEvent)).
//!
//! The hash is recomputed whenever content changes, and every content
//! change raises the version by one. A patch that names a base version
//! other than the current one is not applied: the surface is in conflict
//! (the user sees it is out of date and its actions are refused) until the
//! author sends a change against the current version, or a full copy
//! replaces it. A surface over a renderer limit is stored as its fallback
//! text only, so it never renders partially and never holds oversized data.
//!
//! The versioned protocol events live in `surface_events`.

use std::collections::BTreeMap;
use std::mem;

use serde_json::Value;

use super::Store;
use super::changes::{ApplyError, Changes};
use super::index::presented_grants;
use super::surface_ops::apply_surface_op;
use crate::conformance::{LimitKind, Limits, SurfaceHash, over_limit, surface_hash};
use crate::event::SurfaceOp;
use crate::ids::{BlockId, MomentId, SurfaceId};
use crate::model::{AgentSurfacePayload, BlockKind, MomentKind};

/// The state of one surface version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SurfaceState {
    /// Current and renderable.
    Open,
    /// A change made against another version was not applied; the surface
    /// is out of date until the author sends a change against this one.
    Conflict {
        /// The base the refused change named.
        base_hash: SurfaceHash,
    },
    /// The surface (or a change to it) exceeded a renderer limit: only its
    /// fallback text is shown, nothing can be acted on, and further
    /// changes are refused until it closes.
    OverLimit(LimitKind),
    /// The author closed it: a frozen record.
    Closed,
}

/// A text stream of one block.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StreamState {
    /// Characters streamed into the block so far.
    pub chars: usize,
    /// The stream ended (`final`); further appends are refused.
    pub finished: bool,
}

/// What the store knows about one surface beside its content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceRecord {
    /// Surface id.
    pub surface_id: SurfaceId,
    /// Current version (1 when created).
    pub version: u64,
    /// Content hash of the current version.
    pub hash: SurfaceHash,
    /// State.
    pub state: SurfaceState,
    /// Text streams by block.
    pub streams: BTreeMap<BlockId, StreamState>,
}

impl SurfaceRecord {
    /// Whether the surface takes actions now (open and current).
    pub fn is_actionable(&self) -> bool {
        self.state == SurfaceState::Open
    }

    /// The limit the surface exceeded, if it did.
    pub fn over_limit(&self) -> Option<LimitKind> {
        match self.state {
            SurfaceState::OverLimit(limit) => Some(limit),
            _ => None,
        }
    }
}

/// Whether `kind` takes streamed text.
pub(super) const fn streamable(kind: BlockKind) -> bool {
    matches!(kind, BlockKind::Text | BlockKind::Markdown | BlockKind::LogStream)
}

/// Text streams a new surface opens: its streamable blocks whose text is
/// still empty (they exist to be streamed into).
pub(super) fn open_streams(surface: &AgentSurfacePayload) -> BTreeMap<BlockId, StreamState> {
    surface
        .blocks
        .iter()
        .filter(|b| streamable(b.kind) && b.data.get("text").and_then(Value::as_str) == Some(""))
        .map(|b| (b.id.clone(), StreamState::default()))
        .collect()
}

/// Whether any stream is still open.
pub(super) fn any_open(streams: &BTreeMap<BlockId, StreamState>) -> bool {
    streams.values().any(|s| !s.finished)
}

/// Reduces an over-limit surface to its fallback text (cut to the text
/// limit): nothing partial is kept.
pub(super) fn to_fallback(surface: &mut AgentSurfacePayload) {
    surface.blocks.clear();
    surface.actions.clear();
    surface.streaming = false;
    let max = Limits::DESKTOP.max_text_chars;
    if surface.fallback_text.chars().count() > max {
        surface.fallback_text = surface.fallback_text.chars().take(max - 1).collect();
        surface.fallback_text.push('…');
    }
}

impl Store {
    /// Records a surface moment's (new) payload: a new surface starts at
    /// version 1, a full copy raises the version and resolves a conflict.
    /// A payload over a renderer limit is stored as its fallback only.
    pub(super) fn track_surface(&mut self, id: &MomentId, kind: &mut MomentKind) {
        let MomentKind::AgentSurface(surface) = kind else { return };
        let limit = over_limit(surface, &Limits::DESKTOP);
        if limit.is_some() {
            to_fallback(surface);
        }
        let hash = surface_hash(surface);
        let previous = self.surfaces.get(id);
        let closed = previous.is_some_and(|r| r.state == SurfaceState::Closed);
        let state = match limit {
            _ if closed => SurfaceState::Closed,
            Some(limit) => SurfaceState::OverLimit(limit),
            None => SurfaceState::Open,
        };
        let record = SurfaceRecord {
            surface_id: surface.surface_id.clone(),
            version: previous.map_or(1, |r| r.version.saturating_add(1)),
            hash,
            state,
            streams: previous.map_or_else(|| open_streams(surface), |r| r.streams.clone()),
        };
        if let Some(previous) = previous
            && previous.surface_id != record.surface_id
        {
            let old = previous.surface_id.clone();
            self.surface_ids.remove(&old);
        }
        self.surface_ids.entry(record.surface_id.clone()).or_insert_with(|| id.clone());
        self.surfaces.insert(id.clone(), record);
    }

    /// Forgets a removed moment's surface.
    pub(super) fn forget_surface(&mut self, id: &MomentId) {
        if let Some(record) = self.surfaces.remove(id)
            && self.surface_ids.get(&record.surface_id) == Some(id)
        {
            self.surface_ids.remove(&record.surface_id);
        }
    }

    pub(super) fn patch_surface(
        &mut self,
        moment_id: &MomentId,
        surface_id: &SurfaceId,
        ops: &[SurfaceOp],
        base_hash: Option<&SurfaceHash>,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        let moment = self
            .moments
            .get_mut(moment_id)
            .ok_or_else(|| ApplyError::UnknownMoment(moment_id.clone()))?;
        let mismatch = || ApplyError::SurfaceMismatch {
            moment: moment_id.clone(),
            surface: surface_id.clone(),
        };
        let MomentKind::AgentSurface(surface) = &mut moment.kind else {
            return Err(mismatch());
        };
        if &surface.surface_id != surface_id {
            return Err(mismatch());
        }
        let record = self.surfaces.get_mut(moment_id).ok_or_else(mismatch)?;
        match record.state {
            SurfaceState::Closed => return Err(ApplyError::SurfaceClosed(surface_id.clone())),
            SurfaceState::OverLimit(_) => {
                return Err(ApplyError::SurfaceOverLimit(surface_id.clone()));
            }
            SurfaceState::Open | SurfaceState::Conflict { .. } => {}
        }
        if let Some(base) = base_hash
            && &record.hash != base
        {
            // Never applied silently on a version the sender did not see.
            record.state = SurfaceState::Conflict { base_hash: base.clone() };
            changes.moments.insert(moment_id.clone());
            return Ok(());
        }
        let mut patched = surface.clone();
        for op in ops {
            apply_surface_op(&mut patched, op)?;
        }
        if let Some(limit) = over_limit(&patched, &Limits::DESKTOP) {
            to_fallback(&mut patched);
            record.state = SurfaceState::OverLimit(limit);
        } else {
            record.state = SurfaceState::Open;
        }
        record.hash = surface_hash(&patched);
        record.version = record.version.saturating_add(1);
        let old = MomentKind::AgentSurface(mem::replace(surface, patched));
        if presented_grants(&old) != presented_grants(&moment.kind) {
            self.index.change_kind(moment_id, &old, &moment.kind);
        }
        changes.moments.insert(moment_id.clone());
        Ok(())
    }

    /// The content hash of the surface a moment holds now.
    pub fn surface_hash(&self, id: &MomentId) -> Option<&SurfaceHash> {
        self.surfaces.get(id).map(|r| &r.hash)
    }

    /// Whether a change based on another version left this surface out of
    /// date.
    pub fn surface_conflict(&self, id: &MomentId) -> bool {
        self.surfaces.get(id).is_some_and(|r| matches!(r.state, SurfaceState::Conflict { .. }))
    }

    /// Version, hash, state and streams of the surface a moment holds.
    pub fn surface(&self, id: &MomentId) -> Option<&SurfaceRecord> {
        self.surfaces.get(id)
    }

    /// The moment that presents `surface`.
    pub fn surface_moment(&self, surface: &SurfaceId) -> Option<&MomentId> {
        self.surface_ids.get(surface)
    }
}
