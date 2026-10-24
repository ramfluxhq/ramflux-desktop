// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The versioned surface events: create, patch against a base hash, stream
//! text, close; an authority's snapshot; the authority's answers to the
//! user's actions and update requests.
//!
//! Each arm checks everything before its first mutation, so a rejected
//! event leaves the store untouched. Order of checks for a change to an
//! existing surface: known surface, not closed, not over a limit; then, for
//! a patch, the base hash (a mismatch puts the surface in conflict and is
//! not an error: it is a fact the user sees); then the version, which must
//! be exactly the next one; then the content. A change that would exceed a
//! renderer limit is not applied: the surface shows only its fallback text
//! from then on (never a partial rendering).
//!
//! A snapshot replaces the whole state at its version (it may skip versions
//! and is how a surface first appears when the source reads it from an
//! authority). It is never older than what the store holds.

use std::collections::BTreeSet;

use serde_json::Value;

use super::Store;
use super::changes::{ApplyError, Changes};
use super::index::presented_grants;
use super::surfaces::{
    StreamState, SurfaceRecord, SurfaceState, UpdateRequest, any_open, open_streams, streamable,
    to_fallback,
};
use crate::conformance::{LimitKind, Limits, SurfaceHash, over_limit};
use crate::event::{SurfaceEvent, SurfacePatchOp, SurfaceSnapshot};
use crate::ids::{BlockId, MomentId, SurfaceId, Timestamp};
use crate::model::{AgentSurfacePayload, Block, Lifecycle, Moment, MomentKind};

impl Store {
    pub(super) fn surface_event(
        &mut self,
        at: Timestamp,
        event: &SurfaceEvent,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        if let SurfaceEvent::Create {
            moment_id,
            context_id,
            origin,
            surface_id,
            version,
            blocks,
            actions,
            fallback_text,
            hash,
        } = event
        {
            if *version != 1 {
                return Err(version_error(surface_id, 1, *version));
            }
            if self.surface_ids.contains_key(surface_id) {
                return Err(ApplyError::DuplicateSurface(surface_id.clone()));
            }
            if fallback_text.trim().is_empty() {
                return Err(ApplyError::MissingFallback(surface_id.clone()));
            }
            unique_blocks(blocks)?;
            let surface = AgentSurfacePayload {
                surface_id: surface_id.clone(),
                fallback_text: fallback_text.clone(),
                blocks: blocks.clone(),
                actions: actions.clone(),
                streaming: false,
            };
            let moment = Moment {
                id: moment_id.clone(),
                context_id: context_id.clone(),
                origin: origin.clone(),
                at,
                kind: MomentKind::AgentSurface(surface),
                lifecycle: Lifecycle::Done,
            };
            // Validates the moment and context, records version 1.
            self.add_moment(at, &moment, changes)?;
            if let (Some(hash), Some(record)) = (hash, self.surfaces.get_mut(moment_id)) {
                record.hash = hash.clone();
            }
            return self.open_created_streams(moment_id);
        }
        if let SurfaceEvent::Snapshot(snapshot) = event {
            return self.surface_snapshot(at, snapshot, changes);
        }
        let surface_id = event.surface_id();
        let moment_id = self
            .surface_ids
            .get(surface_id)
            .cloned()
            .ok_or_else(|| ApplyError::UnknownSurface(surface_id.clone()))?;
        let record = self.surfaces.get(&moment_id).cloned().ok_or_else(|| unknown(surface_id))?;
        match event {
            SurfaceEvent::ActionRefused { .. } => {
                changes.surface_refusals.insert(moment_id.clone());
                changes.moments.insert(moment_id);
                return Ok(());
            }
            SurfaceEvent::UpdateRequested { refused, .. } => {
                if let Some(r) = self.surfaces.get_mut(&moment_id) {
                    r.update = Some(match refused {
                        Some(reason) => UpdateRequest::Refused(reason.clone()),
                        None => UpdateRequest::Sent,
                    });
                }
                changes.moments.insert(moment_id);
                return Ok(());
            }
            SurfaceEvent::UpdatedAt { at, .. } => {
                if let Some(r) = self.surfaces.get_mut(&moment_id)
                    && r.updated_at != Some(*at)
                {
                    r.updated_at = Some(*at);
                    changes.moments.insert(moment_id);
                }
                return Ok(());
            }
            SurfaceEvent::Removed { .. } => {
                self.remove_surface_moment(&moment_id, changes);
                return Ok(());
            }
            _ => {}
        }
        match record.state {
            SurfaceState::Closed => return Err(ApplyError::SurfaceClosed(surface_id.clone())),
            SurfaceState::OverLimit(_) if !matches!(event, SurfaceEvent::Close { .. }) => {
                return Err(ApplyError::SurfaceOverLimit(surface_id.clone()));
            }
            _ => {}
        }
        if let SurfaceEvent::Patch { base_hash, .. } = event
            && base_hash != &record.hash
        {
            // The author did not see this version: not applied, and the
            // user sees the surface is out of date.
            if let Some(r) = self.surfaces.get_mut(&moment_id) {
                r.state = SurfaceState::Conflict { base_hash: base_hash.clone() };
            }
            changes.moments.insert(moment_id);
            return Ok(());
        }
        let expected = record.version.saturating_add(1);
        if event.version() != expected {
            return Err(version_error(surface_id, expected, event.version()));
        }
        let current = match self.moments.get(&moment_id).map(|m| &m.kind) {
            Some(MomentKind::AgentSurface(surface)) => surface.clone(),
            _ => return Err(unknown(surface_id)),
        };
        let mut next = record.clone();
        next.version = expected;
        next.update = None;
        let mut content = current;
        let reported: Option<SurfaceHash>;
        match event {
            SurfaceEvent::Patch { ops, hash, .. } => {
                for op in ops {
                    patch(&mut content, op)?;
                }
                reported = hash.clone();
                if next.state != SurfaceState::OutOfSync {
                    next.state = SurfaceState::Open;
                }
                next.streams.retain(|id, _| content.blocks.iter().any(|b| &b.id == id));
            }
            SurfaceEvent::StreamAppend { block_id, text_delta, is_final, hash, .. } => {
                reported = hash.clone();
                if let Some(limit) = stream(&mut content, &mut next, block_id, text_delta)? {
                    next.state = SurfaceState::OverLimit(limit);
                }
                if *is_final {
                    next.streams.entry(block_id.clone()).or_default().finished = true;
                }
            }
            SurfaceEvent::Close { .. } => {
                // Closing changes no content: the hash stays.
                reported = Some(record.hash.clone());
                next.state = SurfaceState::Closed;
                for stream in next.streams.values_mut() {
                    stream.finished = true;
                }
            }
            SurfaceEvent::Create { .. }
            | SurfaceEvent::Snapshot(_)
            | SurfaceEvent::ActionRefused { .. }
            | SurfaceEvent::UpdateRequested { .. }
            | SurfaceEvent::UpdatedAt { .. }
            | SurfaceEvent::Removed { .. } => unreachable!("handled above"),
        }
        if next.over_limit().is_none()
            && next.state != SurfaceState::Closed
            && let Some(limit) = over_limit(&content, &Limits::DESKTOP)
        {
            next.state = SurfaceState::OverLimit(limit);
        }
        if next.over_limit().is_some() {
            // Never a partial rendering: only the fallback text remains.
            // The refused change does not count as a version (its author
            // refuses it the same way), so a later close still lines up.
            to_fallback(&mut content);
            if record.over_limit().is_none() {
                next.version = record.version;
            }
        }
        content.streaming = next.state != SurfaceState::Closed && any_open(&next.streams);
        next.hash = if next.version == record.version {
            record.hash.clone()
        } else {
            reported.unwrap_or_else(|| SurfaceHash::of_version(next.version))
        };
        self.replace_surface(&moment_id, content, next, changes);
        Ok(())
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
        unique_blocks(&snapshot.blocks)?;
        let mut content = AgentSurfacePayload {
            surface_id: surface_id.clone(),
            fallback_text: snapshot.fallback_text.clone(),
            blocks: snapshot.blocks.clone(),
            actions: snapshot.actions.clone(),
            streaming: false,
        };
        let previous = match self.surface_ids.get(surface_id).cloned() {
            Some(moment_id) => {
                if moment_id != snapshot.moment_id {
                    return Err(ApplyError::SurfaceMismatch {
                        moment: snapshot.moment_id.clone(),
                        surface: surface_id.clone(),
                    });
                }
                let record =
                    self.surfaces.get(&moment_id).cloned().ok_or_else(|| unknown(surface_id))?;
                if record.state == SurfaceState::Closed {
                    return Err(ApplyError::SurfaceClosed(surface_id.clone()));
                }
                if snapshot.version < record.version {
                    return Err(version_error(surface_id, record.version, snapshot.version));
                }
                Some(record)
            }
            None => {
                let moment = Moment {
                    id: snapshot.moment_id.clone(),
                    context_id: snapshot.context_id.clone(),
                    origin: snapshot.origin.clone(),
                    at: snapshot.at.unwrap_or(at),
                    kind: MomentKind::AgentSurface(content.clone()),
                    lifecycle: Lifecycle::Done,
                };
                self.add_moment(at, &moment, changes)?;
                None
            }
        };
        let limit = over_limit(&content, &Limits::DESKTOP);
        if limit.is_some() {
            to_fallback(&mut content);
        }
        let open = |id: &BlockId| snapshot.open_streams.contains(id) && !snapshot.closed;
        let streams = content
            .blocks
            .iter()
            .filter(|b| {
                streamable(b.kind)
                    && (open(&b.id)
                        || previous.as_ref().is_some_and(|r| r.streams.contains_key(&b.id)))
            })
            .map(|b| {
                let chars =
                    b.data.get("text").and_then(Value::as_str).map_or(0, |t| t.chars().count());
                (b.id.clone(), StreamState { chars, finished: !open(&b.id) })
            })
            .collect();
        let state = match limit {
            _ if snapshot.closed => SurfaceState::Closed,
            Some(limit) => SurfaceState::OverLimit(limit),
            None if snapshot.out_of_sync => SurfaceState::OutOfSync,
            None => SurfaceState::Open,
        };
        let update = previous.as_ref().and_then(|r| {
            (snapshot.out_of_sync && r.version == snapshot.version)
                .then(|| r.update.clone())
                .flatten()
        });
        let record = SurfaceRecord {
            surface_id: surface_id.clone(),
            version: snapshot.version,
            hash: snapshot.hash.clone(),
            state,
            streams,
            update,
            updated_at: previous.as_ref().and_then(|r| r.updated_at),
        };
        content.streaming = !snapshot.closed && any_open(&record.streams);
        self.replace_surface(&snapshot.moment_id, content, record, changes);
        Ok(())
    }

    /// A created surface streams into its empty text blocks.
    fn open_created_streams(&mut self, moment_id: &MomentId) -> Result<(), ApplyError> {
        let Some(Moment { kind: MomentKind::AgentSurface(surface), .. }) =
            self.moments.get_mut(moment_id)
        else {
            return Ok(());
        };
        let Some(record) = self.surfaces.get_mut(moment_id) else { return Ok(()) };
        if record.over_limit().is_some() {
            return Ok(());
        }
        record.streams = open_streams(surface);
        surface.streaming = any_open(&record.streams);
        Ok(())
    }

    fn replace_surface(
        &mut self,
        moment_id: &MomentId,
        content: AgentSurfacePayload,
        record: SurfaceRecord,
        changes: &mut Changes,
    ) {
        if let Some(moment) = self.moments.get_mut(moment_id) {
            let new = MomentKind::AgentSurface(content);
            if presented_grants(&moment.kind) != presented_grants(&new) {
                self.index.change_kind(moment_id, &moment.kind, &new);
            }
            moment.kind = new;
        }
        self.surfaces.insert(moment_id.clone(), record);
        changes.moments.insert(moment_id.clone());
    }
}

fn unknown(surface: &SurfaceId) -> ApplyError {
    ApplyError::UnknownSurface(surface.clone())
}

fn version_error(surface: &SurfaceId, expected: u64, found: u64) -> ApplyError {
    ApplyError::SurfaceVersion { surface: surface.clone(), expected, found }
}

fn unique_blocks(blocks: &[Block]) -> Result<(), ApplyError> {
    let mut seen = BTreeSet::new();
    match blocks.iter().find(|b| !seen.insert(&b.id)) {
        Some(dup) => Err(ApplyError::DuplicateBlock(dup.id.clone())),
        None => Ok(()),
    }
}

fn patch(content: &mut AgentSurfacePayload, op: &SurfacePatchOp) -> Result<(), ApplyError> {
    match op {
        SurfacePatchOp::ReplaceBlock { block } => {
            let slot = content
                .blocks
                .iter_mut()
                .find(|b| b.id == block.id)
                .ok_or_else(|| ApplyError::UnknownBlock(block.id.clone()))?;
            *slot = block.clone();
        }
        SurfacePatchOp::InsertBlock { index, block } => {
            if *index > content.blocks.len() {
                return Err(ApplyError::InsertOutOfRange {
                    block: block.id.clone(),
                    index: *index,
                });
            }
            if content.blocks.iter().any(|b| b.id == block.id) {
                return Err(ApplyError::DuplicateBlock(block.id.clone()));
            }
            content.blocks.insert(*index, block.clone());
        }
        SurfacePatchOp::RemoveBlock { block_id } => {
            let ix = content
                .blocks
                .iter()
                .position(|b| &b.id == block_id)
                .ok_or_else(|| ApplyError::UnknownBlock(block_id.clone()))?;
            content.blocks.remove(ix);
        }
        SurfacePatchOp::SetActions { actions } => content.actions.clone_from(actions),
        SurfacePatchOp::SetFallback { fallback_text } => {
            if fallback_text.trim().is_empty() {
                return Err(ApplyError::MissingFallback(content.surface_id.clone()));
            }
            content.fallback_text.clone_from(fallback_text);
        }
    }
    Ok(())
}

/// Appends `delta` to the text of `block_id`. Returns the limit the stream
/// exceeded (the append is then not made), or `None` when it fits.
fn stream(
    content: &mut AgentSurfacePayload,
    record: &mut SurfaceRecord,
    block_id: &BlockId,
    delta: &str,
) -> Result<Option<LimitKind>, ApplyError> {
    let block = content
        .blocks
        .iter_mut()
        .find(|b| &b.id == block_id)
        .ok_or_else(|| ApplyError::UnknownBlock(block_id.clone()))?;
    if record.streams.get(block_id).is_some_and(|s| s.finished) {
        return Err(ApplyError::StreamFinished(block_id.clone()));
    }
    let limits = Limits::DESKTOP;
    let text = match block.data.get_mut("text") {
        Some(Value::String(text)) if streamable(block.kind) => text,
        _ => return Err(ApplyError::NotTextBlock(block_id.clone())),
    };
    if delta.len() > limits.max_text_delta_bytes {
        return Ok(Some(LimitKind::TextDelta));
    }
    let chars = text.chars().count() + delta.chars().count();
    if chars > limits.max_stream_chars {
        return Ok(Some(LimitKind::StreamLength));
    }
    text.push_str(delta);
    record.streams.entry(block_id.clone()).or_default().chars = chars;
    Ok(None)
}
