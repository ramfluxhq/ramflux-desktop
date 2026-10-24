// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Agent surface versions: what the store knows about every surface beside
//! its content (version, content hash, state, open text streams), and the
//! unversioned paths that change a surface (a full copy in a moment
//! payload, [`DomainEvent::SurfacePatched`](crate::DomainEvent)).
//!
//! Every content change raises the version by one; its hash is the one the
//! source reported, or, for a surface whose source reports none (an
//! unversioned payload), the version label [`SurfaceHash::of_version`]. A patch that names a base version
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
use crate::conformance::{LimitKind, Limits, SurfaceHash, over_limit};
use crate::event::SurfaceOp;
use crate::ids::{BlockId, MomentId, SurfaceId, Timestamp};
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
    /// The authority holding the surface knows it missed a change of the
    /// author's: what is shown is not current, and nothing can be acted on
    /// until it catches up (the user can ask the author for the current
    /// state).
    OutOfSync,
}

/// The user's request for the author's current state of an out-of-sync
/// surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateRequest {
    /// Sent; the author answers with its current state.
    Sent,
    /// Could not be sent, with the reason.
    Refused(String),
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
    /// The user's request for the current state, while it is relevant.
    pub update: Option<UpdateRequest>,
    /// When the author last changed it, as the authority reports (the
    /// author's clock; display only).
    pub updated_at: Option<Timestamp>,
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
        let previous = self.surfaces.get(id);
        let version = previous.map_or(1, |r| r.version.saturating_add(1));
        let closed = previous.is_some_and(|r| r.state == SurfaceState::Closed);
        let state = match limit {
            _ if closed => SurfaceState::Closed,
            Some(limit) => SurfaceState::OverLimit(limit),
            None => SurfaceState::Open,
        };
        let record = SurfaceRecord {
            surface_id: surface.surface_id.clone(),
            version,
            hash: SurfaceHash::of_version(version),
            state,
            streams: previous.map_or_else(|| open_streams(surface), |r| r.streams.clone()),
            update: None,
            updated_at: previous.and_then(|r| r.updated_at),
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

    /// Removes a surface's moment (its authority no longer holds it).
    pub(super) fn remove_surface_moment(&mut self, id: &MomentId, changes: &mut Changes) {
        if let Some(moment) = self.moments.remove(id) {
            let since = self.needs_me_since.remove(id);
            self.index.remove_moment(&moment, since);
            changes.contexts.insert(moment.context_id);
        }
        self.bodies.remove(id);
        self.forget_surface(id);
        changes.moments.insert(id.clone());
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
            SurfaceState::Open | SurfaceState::Conflict { .. } | SurfaceState::OutOfSync => {}
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
        } else if record.state != SurfaceState::OutOfSync {
            record.state = SurfaceState::Open;
        }
        // The streaming flag is not content: a change of only it keeps the
        // version (actions shown on it stay current).
        let mut unflagged = patched.clone();
        unflagged.streaming = surface.streaming;
        if unflagged != *surface || record.over_limit().is_some() {
            record.version = record.version.saturating_add(1);
            record.hash = SurfaceHash::of_version(record.version);
        }
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

    /// Whether the authority holding this surface knows it is behind the
    /// author.
    pub fn surface_out_of_sync(&self, id: &MomentId) -> bool {
        self.surfaces.get(id).is_some_and(|r| r.state == SurfaceState::OutOfSync)
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
