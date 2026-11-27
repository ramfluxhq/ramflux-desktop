// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Agent surface versions: what the store knows about every surface beside
//! its fallback text and actions (version, content hash, state), and the
//! facts the authority reports about it (`surface_events`).
//!
//! Actions are bound to the version and hash they were shown on. A surface
//! the author closed is a frozen record.

use super::Store;
use super::changes::Changes;
use crate::ids::{MomentId, SurfaceId, Timestamp};
use crate::model::MomentKind;
use crate::surface_hash::SurfaceHash;

/// The state of one surface version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceState {
    /// Current; its actions can be taken.
    Open,
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

/// What the store knows about one surface beside its payload.
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
}

impl Store {
    /// Forgets a removed moment's surface and its card.
    pub(super) fn forget_surface(&mut self, id: &MomentId) {
        if let Some(record) = self.surfaces.remove(id) {
            if self.surface_ids.get(&record.surface_id) == Some(id) {
                self.surface_ids.remove(&record.surface_id);
            }
            self.card_records.remove(&record.surface_id);
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

    /// The content hash of the surface a moment holds now.
    pub fn surface_hash(&self, id: &MomentId) -> Option<&SurfaceHash> {
        self.surfaces.get(id).map(|r| &r.hash)
    }

    /// Whether the authority holding this surface knows it is behind the
    /// author.
    pub fn surface_out_of_sync(&self, id: &MomentId) -> bool {
        self.surfaces.get(id).is_some_and(|r| r.state == SurfaceState::OutOfSync)
    }

    /// Version, hash and state of the surface a moment holds.
    pub fn surface(&self, id: &MomentId) -> Option<&SurfaceRecord> {
        self.surfaces.get(id)
    }

    /// The moment that presents `surface`.
    pub fn surface_moment(&self, surface: &SurfaceId) -> Option<&MomentId> {
        self.surface_ids.get(surface)
    }

    /// Records a surface moment's payload as the store holds it: a new
    /// surface starts at version 1; the version otherwise moves only with
    /// the authority's snapshot (see `surface_events`).
    pub(super) fn track_surface(&mut self, id: &MomentId, kind: &MomentKind) {
        let MomentKind::AgentSurface(surface) = kind else { return };
        let previous = self.surfaces.get(id);
        let record = SurfaceRecord {
            surface_id: surface.surface_id.clone(),
            version: previous.map_or(1, |r| r.version),
            hash: previous.map_or_else(|| SurfaceHash::of_version(1), |r| r.hash.clone()),
            state: previous.map_or(SurfaceState::Open, |r| r.state),
            update: previous.and_then(|r| r.update.clone()),
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
}
