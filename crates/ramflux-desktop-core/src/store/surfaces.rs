// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Agent surface versions: the content hash of every surface in the store,
//! patches with compare-and-swap on that hash, and surface conflicts.
//!
//! The hash is recomputed whenever a surface's content changes (added,
//! replaced, patched). A patch that names a base version other than the
//! current one is not applied: the surface is marked in conflict (the user
//! sees it is out of date and its actions are refused) until a full copy of
//! the surface replaces it.

use std::mem;

use super::Store;
use super::changes::{ApplyError, Changes};
use super::index::presented_grants;
use super::surface_ops::apply_surface_op;
use crate::conformance::{SurfaceHash, surface_hash};
use crate::event::SurfaceOp;
use crate::ids::{MomentId, SurfaceId};
use crate::model::MomentKind;

impl Store {
    /// Records the content hash of a surface moment's (new) payload. A full
    /// copy resolves a conflict.
    pub(super) fn track_surface(&mut self, id: &MomentId, kind: &MomentKind) {
        if let MomentKind::AgentSurface(surface) = kind {
            self.surface_hashes.insert(id.clone(), surface_hash(surface));
            self.surface_conflicts.remove(id);
        }
    }

    /// Forgets a removed moment's surface version.
    pub(super) fn forget_surface(&mut self, id: &MomentId) {
        self.surface_hashes.remove(id);
        self.surface_conflicts.remove(id);
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
        if let Some(base) = base_hash
            && self.surface_hashes.get(moment_id) != Some(base)
        {
            // Never applied silently on a version the sender did not see.
            self.surface_conflicts.insert(moment_id.clone());
            changes.moments.insert(moment_id.clone());
            return Ok(());
        }
        let mut patched = surface.clone();
        for op in ops {
            apply_surface_op(&mut patched, op)?;
        }
        let hash = surface_hash(&patched);
        let old = MomentKind::AgentSurface(mem::replace(surface, patched));
        if presented_grants(&old) != presented_grants(&moment.kind) {
            self.index.change_kind(moment_id, &old, &moment.kind);
        }
        self.surface_hashes.insert(moment_id.clone(), hash);
        changes.moments.insert(moment_id.clone());
        Ok(())
    }

    /// The content hash of the surface a moment holds now.
    pub fn surface_hash(&self, id: &MomentId) -> Option<&SurfaceHash> {
        self.surface_hashes.get(id)
    }

    /// Whether a patch based on another version left this surface out of
    /// date (until a full copy replaces it).
    pub fn surface_conflict(&self, id: &MomentId) -> bool {
        self.surface_conflicts.contains(id)
    }
}
