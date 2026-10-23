// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Agent surface patch operations.

use serde_json::Value;

use super::changes::ApplyError;
use crate::event::SurfaceOp;
use crate::model::AgentSurfacePayload;

/// Applies one operation to a working copy of a surface.
pub(super) fn apply_surface_op(
    surface: &mut AgentSurfacePayload,
    op: &SurfaceOp,
) -> Result<(), ApplyError> {
    match op {
        SurfaceOp::AppendBlocks(blocks) => surface.blocks.extend(blocks.iter().cloned()),
        SurfaceOp::ReplaceBlock(block) => {
            let slot = surface
                .blocks
                .iter_mut()
                .find(|b| b.id == block.id)
                .ok_or_else(|| ApplyError::UnknownBlock(block.id.clone()))?;
            *slot = block.clone();
        }
        SurfaceOp::ReplaceBlocks(blocks) => surface.blocks.clone_from(blocks),
        SurfaceOp::AppendText { block_id, text } => {
            let block = surface
                .blocks
                .iter_mut()
                .find(|b| &b.id == block_id)
                .ok_or_else(|| ApplyError::UnknownBlock(block_id.clone()))?;
            match block.data.get_mut("text") {
                Some(Value::String(current)) => current.push_str(text),
                _ => return Err(ApplyError::NotTextBlock(block_id.clone())),
            }
        }
        SurfaceOp::SetActions(actions) => surface.actions.clone_from(actions),
        SurfaceOp::SetStreaming(streaming) => surface.streaming = *streaming,
    }
    Ok(())
}
