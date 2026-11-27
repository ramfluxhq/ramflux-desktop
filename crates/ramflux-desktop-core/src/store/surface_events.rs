// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The surface facts the authority reports: a snapshot of a surface's state
//! (it creates the surface's moment the first time), the authority's answers
//! to the user's actions and update requests, its last-updated time, and its
//! removal.
//!
//! Each arm checks everything before its first mutation, so a rejected event
//! leaves the store untouched. A snapshot is never older than what the store
//! holds, and a closed surface is frozen.

use super::Store;
use super::changes::{ApplyError, Changes};
use super::surfaces::{SurfaceRecord, SurfaceState, UpdateRequest};
use crate::event::SurfaceEvent;
use crate::ids::{MomentId, SurfaceId, Timestamp};
use crate::model::{AgentSurfacePayload, Lifecycle, Moment, MomentKind};
use crate::surface_event::SurfaceSnapshot;

impl Store {
    pub(super) fn surface_event(
        &mut self,
        at: Timestamp,
        event: &SurfaceEvent,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        match event {
            SurfaceEvent::Snapshot(snapshot) => self.surface_snapshot(at, snapshot, changes),
            SurfaceEvent::ActionRefused { surface_id, .. } => {
                let moment = self.moment_of_surface(surface_id)?;
                changes.surface_refusals.insert(moment.clone());
                changes.moments.insert(moment);
                Ok(())
            }
            SurfaceEvent::UpdateRequested { surface_id, refused } => {
                let moment = self.moment_of_surface(surface_id)?;
                if let Some(record) = self.surfaces.get_mut(&moment) {
                    record.update = Some(match refused {
                        Some(reason) => UpdateRequest::Refused(reason.clone()),
                        None => UpdateRequest::Sent,
                    });
                }
                changes.moments.insert(moment);
                Ok(())
            }
            SurfaceEvent::UpdatedAt { surface_id, at } => {
                let moment = self.moment_of_surface(surface_id)?;
                if let Some(record) = self.surfaces.get_mut(&moment)
                    && record.updated_at != Some(*at)
                {
                    record.updated_at = Some(*at);
                    changes.moments.insert(moment);
                }
                Ok(())
            }
            SurfaceEvent::Removed { surface_id } => {
                let moment = self.moment_of_surface(surface_id)?;
                self.remove_surface_moment(&moment, changes);
                Ok(())
            }
        }
    }

    fn moment_of_surface(&self, surface: &SurfaceId) -> Result<MomentId, ApplyError> {
        self.surface_ids
            .get(surface)
            .cloned()
            .ok_or_else(|| ApplyError::UnknownSurface(surface.clone()))
    }

    /// Applies an authority's whole state of a surface; creates its moment
    /// when the surface is new.
    fn surface_snapshot(
        &mut self,
        at: Timestamp,
        snapshot: &SurfaceSnapshot,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        let surface_id = &snapshot.surface_id;
        if snapshot.version == 0 {
            return Err(version_error(surface_id, 1, 0));
        }
        if snapshot.fallback_text.trim().is_empty() {
            return Err(ApplyError::MissingFallback(surface_id.clone()));
        }
        let content = AgentSurfacePayload {
            surface_id: surface_id.clone(),
            fallback_text: snapshot.fallback_text.clone(),
            actions: snapshot.actions.clone(),
        };
        let state = if snapshot.closed {
            SurfaceState::Closed
        } else if snapshot.out_of_sync {
            SurfaceState::OutOfSync
        } else {
            SurfaceState::Open
        };
        match self.surface_ids.get(surface_id).cloned() {
            Some(moment_id) => {
                if moment_id != snapshot.moment_id {
                    return Err(ApplyError::SurfaceMismatch {
                        moment: snapshot.moment_id.clone(),
                        surface: surface_id.clone(),
                    });
                }
                let previous =
                    self.surfaces.get(&moment_id).cloned().ok_or_else(|| unknown(surface_id))?;
                if previous.state == SurfaceState::Closed {
                    return Err(ApplyError::SurfaceClosed(surface_id.clone()));
                }
                if snapshot.version < previous.version {
                    return Err(version_error(surface_id, previous.version, snapshot.version));
                }
                let moment = self
                    .moments
                    .get_mut(&moment_id)
                    .ok_or_else(|| ApplyError::UnknownMoment(moment_id.clone()))?;
                moment.kind = MomentKind::AgentSurface(content);
                let record = SurfaceRecord {
                    version: snapshot.version,
                    hash: snapshot.hash.clone(),
                    state,
                    update: keep_update(&previous, snapshot),
                    ..previous
                };
                self.surfaces.insert(moment_id.clone(), record);
                changes.moments.insert(moment_id);
            }
            None => {
                let moment = Moment {
                    order: snapshot.order.clone(),
                    id: snapshot.moment_id.clone(),
                    context_id: snapshot.context_id.clone(),
                    origin: snapshot.origin.clone(),
                    at: snapshot.at.unwrap_or(at),
                    kind: MomentKind::AgentSurface(content),
                    lifecycle: Lifecycle::Done,
                };
                self.add_moment(at, &moment, changes)?;
                let record = SurfaceRecord {
                    surface_id: surface_id.clone(),
                    version: snapshot.version,
                    hash: snapshot.hash.clone(),
                    state,
                    update: None,
                    updated_at: None,
                };
                self.surfaces.insert(snapshot.moment_id.clone(), record);
            }
        }
        Ok(())
    }
}

/// The user's pending update request survives a snapshot of the same
/// version that is still out of sync.
fn keep_update(previous: &SurfaceRecord, snapshot: &SurfaceSnapshot) -> Option<UpdateRequest> {
    if snapshot.out_of_sync && previous.version == snapshot.version {
        previous.update.clone()
    } else {
        None
    }
}

fn unknown(surface: &SurfaceId) -> ApplyError {
    ApplyError::UnknownSurface(surface.clone())
}

fn version_error(surface: &SurfaceId, expected: u64, found: u64) -> ApplyError {
    ApplyError::SurfaceVersion { surface: surface.clone(), expected, found }
}
