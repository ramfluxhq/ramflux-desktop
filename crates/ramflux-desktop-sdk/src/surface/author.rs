// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Desktop surface events as an author's protocol events (fixtures and
//! replays: a scenario's AI participant played through a daemon).
//!
//! [`Author`] keeps what it authored per surface (content and the protocol
//! hash of every version), so a fixture's patch can name its base as the
//! version label `vN` and the protocol event carries that version's real
//! hash: a patch against an older version becomes a real base conflict.
//! A change counts only once the daemon accepted it.

use std::collections::BTreeMap;

use ramflux_desktop_core::{AgentSurfacePayload, SurfaceEvent, SurfacePatchOp};
use ramflux_protocol::{
    A2uiConversationCommand, DeleteSurfaceCommand, DeleteSurfaceReason, PatchSurfaceCommand,
    StreamAppendCommand, SurfacePatchOp as WireOp, encode_a2ui_conversation_event,
    surface_snapshot_hash,
};
use serde_json::Value;

use super::catalog::{allowed_action_of, command_of, component_of};

#[derive(Debug, Clone)]
struct Authored {
    payload: AgentSurfacePayload,
    hashes: BTreeMap<u64, String>,
}

/// An author of surfaces in one conversation.
#[derive(Debug, Default)]
pub struct Author {
    surfaces: BTreeMap<String, Authored>,
}

fn hash_of(id: &str, payload: &AgentSurfacePayload) -> Result<String, String> {
    surface_snapshot_hash(&command_of(id, payload)).map_err(|e| e.to_string())
}

fn apply(payload: &mut AgentSurfacePayload, op: &SurfacePatchOp) {
    match op {
        SurfacePatchOp::ReplaceBlock { block } => {
            if let Some(slot) = payload.blocks.iter_mut().find(|b| b.id == block.id) {
                *slot = block.clone();
            }
        }
        SurfacePatchOp::InsertBlock { index, block } => {
            payload.blocks.insert((*index).min(payload.blocks.len()), block.clone());
        }
        SurfacePatchOp::RemoveBlock { block_id } => payload.blocks.retain(|b| &b.id != block_id),
        SurfacePatchOp::SetActions { actions } => payload.actions.clone_from(actions),
        SurfacePatchOp::SetFallback { fallback_text } => {
            payload.fallback_text.clone_from(fallback_text);
        }
    }
}

fn wire_op(op: &SurfacePatchOp) -> WireOp {
    match op {
        SurfacePatchOp::ReplaceBlock { block } => {
            WireOp::ReplaceBlock { component: component_of(block) }
        }
        SurfacePatchOp::InsertBlock { index, block } => {
            WireOp::InsertBlock { index: *index, component: component_of(block) }
        }
        SurfacePatchOp::RemoveBlock { block_id } => {
            WireOp::RemoveBlock { component_id: block_id.to_string() }
        }
        SurfacePatchOp::SetActions { actions } => {
            WireOp::SetActions { actions: actions.iter().map(allowed_action_of).collect() }
        }
        SurfacePatchOp::SetFallback { fallback_text } => {
            WireOp::SetFallbackText { fallback_text: fallback_text.clone() }
        }
    }
}

impl Author {
    /// The protocol hash of a version this author made.
    pub fn hash(&self, surface_id: &str, version: u64) -> Option<&str> {
        self.surfaces.get(surface_id)?.hashes.get(&version).map(String::as_str)
    }

    /// Writes `event` as the protocol event of `conversation`, hands its
    /// bytes to `deliver` (the daemon; `true` when it accepted them) and
    /// records the change when accepted. Events other than create, patch,
    /// stream and close are not an author's.
    ///
    /// # Errors
    ///
    /// Returns why the event cannot be written (an unknown surface, a value
    /// that is not I-JSON).
    pub fn send(
        &mut self,
        conversation: &str,
        event: &SurfaceEvent,
        deliver: impl FnOnce(&[u8]) -> bool,
    ) -> Result<bool, String> {
        let id = event.surface_id().to_string();
        let mut next = match event {
            SurfaceEvent::Create { blocks, actions, fallback_text, .. } => Authored {
                payload: AgentSurfacePayload {
                    surface_id: event.surface_id().clone(),
                    fallback_text: fallback_text.clone(),
                    blocks: blocks.clone(),
                    actions: actions.clone(),
                    streaming: false,
                },
                hashes: BTreeMap::new(),
            },
            _ => self.surfaces.get(&id).cloned().ok_or_else(|| format!("unknown surface {id}"))?,
        };
        let command = match event {
            SurfaceEvent::Create { .. } => {
                A2uiConversationCommand::CreateSurface(command_of(&id, &next.payload))
            }
            SurfaceEvent::Patch { base_hash, ops, .. } => {
                let base = base_hash
                    .as_str()
                    .strip_prefix('v')
                    .and_then(|v| v.parse::<u64>().ok())
                    .and_then(|v| next.hashes.get(&v).cloned())
                    .unwrap_or_else(|| base_hash.to_string());
                for op in ops {
                    apply(&mut next.payload, op);
                }
                A2uiConversationCommand::PatchSurface(PatchSurfaceCommand {
                    surface_id: id.clone(),
                    base_surface_hash: base,
                    ops: ops.iter().map(wire_op).collect(),
                    new_surface_hash: hash_of(&id, &next.payload)?,
                })
            }
            SurfaceEvent::StreamAppend { block_id, text_delta, is_final, .. } => {
                if let Some(block) = next.payload.blocks.iter_mut().find(|b| &b.id == block_id)
                    && let Some(Value::String(text)) = block.data.get_mut("text")
                {
                    text.push_str(text_delta);
                }
                A2uiConversationCommand::StreamAppend(StreamAppendCommand {
                    surface_id: id.clone(),
                    component_id: block_id.to_string(),
                    text_delta: text_delta.clone(),
                    final_: *is_final,
                })
            }
            SurfaceEvent::Close { .. } => {
                A2uiConversationCommand::DeleteSurface(DeleteSurfaceCommand {
                    surface_id: id.clone(),
                    reason: Some(DeleteSurfaceReason::Completed),
                })
            }
            _ => return Err("not an author's surface event".to_owned()),
        };
        let bytes = encode_a2ui_conversation_event(conversation, event.version(), &command)
            .map_err(|e| e.to_string())?;
        if !deliver(&bytes) {
            return Ok(false);
        }
        next.hashes.insert(event.version(), hash_of(&id, &next.payload)?);
        self.surfaces.insert(id, next);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use ramflux_desktop_core::{
        Block, BlockId, BlockKind, ContextId, MomentId, ParticipantId, SurfaceHash, SurfaceId,
    };
    use ramflux_protocol::{A2uiConversationEvent, SurfaceState};
    use serde_json::json;

    use super::*;

    #[test]
    fn authored_events_apply_on_the_protocol_state_machine() {
        let mut author = Author::default();
        let mut state = SurfaceState::default();
        let mut deliver = |bytes: &[u8]| {
            let event = A2uiConversationEvent::from_wire_bytes(bytes).expect("event");
            match state.apply(&event) {
                Ok(next) => {
                    state = next;
                    true
                }
                Err(_) => false,
            }
        };
        let create = SurfaceEvent::Create {
            moment_id: MomentId::new("m"),
            context_id: ContextId::new("c"),
            origin: ParticipantId::new("p"),
            surface_id: SurfaceId::new("s"),
            version: 1,
            blocks: vec![Block::new("b", BlockKind::Markdown, json!({ "text": "" }))],
            actions: Vec::new(),
            fallback_text: "Reply".into(),
            hash: None,
        };
        assert!(author.send("c", &create, &mut deliver).expect("create"));
        let append = SurfaceEvent::StreamAppend {
            surface_id: SurfaceId::new("s"),
            version: 2,
            block_id: BlockId::new("b"),
            text_delta: "Hi".into(),
            is_final: true,
            hash: None,
        };
        assert!(author.send("c", &append, &mut deliver).expect("append"));
        let patch = |base: &str| SurfaceEvent::Patch {
            surface_id: SurfaceId::new("s"),
            version: 3,
            base_hash: SurfaceHash::new(base),
            ops: vec![SurfacePatchOp::SetFallback { fallback_text: "Done".into() }],
            hash: None,
        };
        assert!(!author.send("c", &patch("v1"), &mut deliver).expect("stale base"), "a conflict");
        assert!(author.send("c", &patch("v2"), &mut deliver).expect("current base"));
        assert_eq!(state.hash(), author.hash("s", 3));
        assert_eq!(state.content().map(|c| c.fallback_text.as_str()), Some("Done"));
    }
}
