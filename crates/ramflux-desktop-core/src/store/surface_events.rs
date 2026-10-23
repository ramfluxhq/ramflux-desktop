// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The versioned surface events: create, patch against a base hash, stream
//! text, close.
//!
//! Each arm checks everything before its first mutation, so a rejected
//! event leaves the store untouched. Order of checks for a change to an
//! existing surface: known surface, not closed, not over a limit; then, for
//! a patch, the base hash (a mismatch puts the surface in conflict and is
//! not an error: it is a fact the user sees); then the version, which must
//! be exactly the next one; then the content. A change that would exceed a
//! renderer limit is not applied: the surface shows only its fallback text
//! from then on (never a partial rendering).

use std::collections::BTreeSet;

use serde_json::Value;

use super::Store;
use super::changes::{ApplyError, Changes};
use super::index::presented_grants;
use super::surfaces::{
    SurfaceRecord, SurfaceState, any_open, open_streams, streamable, to_fallback,
};
use crate::conformance::{LimitKind, Limits, over_limit, surface_hash};
use crate::event::{SurfaceEvent, SurfacePatchOp};
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
            return self.open_created_streams(moment_id);
        }
        let surface_id = event.surface_id();
        let moment_id = self
            .surface_ids
            .get(surface_id)
            .cloned()
            .ok_or_else(|| ApplyError::UnknownSurface(surface_id.clone()))?;
        let record = self.surfaces.get(&moment_id).cloned().ok_or_else(|| unknown(surface_id))?;
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
        let mut content = current;
        match event {
            SurfaceEvent::Patch { ops, .. } => {
                for op in ops {
                    patch(&mut content, op)?;
                }
                next.state = SurfaceState::Open;
                next.streams.retain(|id, _| content.blocks.iter().any(|b| &b.id == id));
            }
            SurfaceEvent::StreamAppend { block_id, text_delta, is_final, .. } => {
                if let Some(limit) = stream(&mut content, &mut next, block_id, text_delta)? {
                    next.state = SurfaceState::OverLimit(limit);
                }
                if *is_final {
                    next.streams.entry(block_id.clone()).or_default().finished = true;
                }
            }
            SurfaceEvent::Close { .. } => {
                next.state = SurfaceState::Closed;
                for stream in next.streams.values_mut() {
                    stream.finished = true;
                }
            }
            SurfaceEvent::Create { .. } => unreachable!("handled above"),
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
        next.hash = surface_hash(&content);
        self.replace_surface(&moment_id, content, next, changes);
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
