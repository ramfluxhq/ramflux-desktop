// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Agent surfaces the daemon holds, as surface moments.
//!
//! The daemon reports that a surface changed (`surface.changed`, metadata
//! only) or lists a conversation's surfaces (`surface.list`); the source then
//! reads the surface (`surface.get`) and reports what changed since the
//! state it last reported, as the store's surface events:
//!
//! - a surface seen for the first time, a version jump (reads are
//!   coalesced, so versions can be skipped), a change of the daemon's
//!   out-of-sync flag, or a change the next events cannot express: a
//!   snapshot of the whole state;
//! - the next version that only appends to one text block, or only ends a
//!   block's stream: a stream append (final when the daemon says the
//!   block's stream is final);
//! - the next version that closes the surface and changes nothing else: a
//!   close;
//! - any other next version: a patch against the previous version's hash.
//!
//! Every event carries the daemon's hash. Surfaces are keyed by the
//! daemon's scoped `surface_ref`, which is also the desktop's surface id:
//! two authors may use the same surface id in one conversation.
//!
//! Streams: the daemon reports each text block's stream as `open` (appends
//! permitted) or `final`. Open is a capability, not evidence that the author
//! is writing, so a block is shown as being written while it is open and
//! either was created empty or has grown since it was shown; the mark ends
//! exactly when the daemon reports the block final (or the surface closed).
//! A block reported final is never shown as written again, and once the
//! author has ended a stream, a block it created empty and has not written
//! into is not waited for any more (the daemon keeps it open only as a
//! capability). Every snapshot carries the blocks the daemon reports final.
//!
//! Times: the daemon's `posted_at` (the author's signed posting time) dates
//! the surface moment; for a surface without one, the earliest surface
//! message in the conversation's message list, else the time it was first
//! seen. `updated_at` is reported after each change. Both are the author's
//! clock, for display only.
//!
//! A surface the daemon no longer holds (`surface.changed` with `removed`,
//! then a read that finds nothing) is removed.
//!
//! Surface messages in a conversation's message list carry the surface's
//! fallback for terminal clients; here they are not messages (the surface
//! moment is).

use std::collections::{BTreeMap, BTreeSet};

use ramflux_desktop_core::{
    AgentSurfacePayload, Block, BlockId, BlockKind, ContextId, DomainEvent, MomentId,
    ParticipantId, SurfaceEvent, SurfaceHash, SurfaceId, SurfacePatchOp, SurfaceSnapshot,
    Timestamp,
};
use serde_json::Value;

use super::{Translator, list, seconds, text};
use crate::plan::fetch::Fetch;
use crate::surface::{SurfaceTarget, catalog};

/// What the source last reported about one surface.
#[derive(Debug, Clone)]
pub(crate) struct KnownSurface {
    pub(crate) target: SurfaceTarget,
    context: ContextId,
    version: u64,
    hash: SurfaceHash,
    payload: AgentSurfacePayload,
    closed: bool,
    out_of_sync: bool,
    /// The daemon's last renderer error (metadata only, for change checks).
    renderer_error: Value,
    /// The daemon's stream states (metadata, for change checks).
    streams: Streams,
    /// Its last-updated time, as reported.
    updated_at: Option<Timestamp>,
    /// Blocks reported as being written.
    streaming: BTreeSet<BlockId>,
}

/// The daemon's stream state of each text block: `true` when final.
type Streams = BTreeMap<String, bool>;

/// Reads `streams` (`{component_id: "open" | "final"}`). A block the map
/// does not name takes no appends.
fn streams_of(value: &Value) -> Streams {
    value["streams"]
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(id, state)| match state.as_str() {
                    Some("open") => Some((id.clone(), false)),
                    Some("final") => Some((id.clone(), true)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The surface moment of `surface_ref` in `context`.
pub(crate) fn surface_moment_id(context: &ContextId, surface_ref: &str) -> MomentId {
    MomentId::new(format!("surface:{context}:{surface_ref}"))
}

/// The carrier prefix of a surface message's stored body (never shown).
const CARRIER_PREFIX: &[u8] = b"ramflux.surface.v2\n";

/// Whether a stored message row is a surface message.
pub(crate) fn is_surface_row(record: &Value) -> bool {
    if text(record, "content_type") == "surface" {
        return true;
    }
    record["encrypted_body"].as_array().is_some_and(|bytes| {
        bytes.len() >= CARRIER_PREFIX.len()
            && bytes
                .iter()
                .zip(CARRIER_PREFIX)
                .all(|(byte, expected)| byte.as_u64() == Some(u64::from(*expected)))
    })
}

fn streamable(kind: BlockKind) -> bool {
    matches!(kind, BlockKind::Text | BlockKind::Markdown | BlockKind::LogStream)
}

fn block_text(block: &Block) -> Option<&str> {
    block.data.get("text").and_then(Value::as_str)
}

/// The one streamable block whose text `new` extends, everything else
/// equal: its id and the appended text.
fn appended(old: &AgentSurfacePayload, new: &AgentSurfacePayload) -> Option<(BlockId, String)> {
    if old.fallback_text != new.fallback_text
        || old.actions != new.actions
        || old.blocks.len() != new.blocks.len()
    {
        return None;
    }
    let mut grown = None;
    for (a, b) in old.blocks.iter().zip(&new.blocks) {
        if a == b {
            continue;
        }
        let (Some(before), Some(after)) = (block_text(a), block_text(b)) else { return None };
        let mut rest_a = a.clone();
        let mut rest_b = b.clone();
        rest_a.data["text"] = Value::Null;
        rest_b.data["text"] = Value::Null;
        if grown.is_some()
            || rest_a != rest_b
            || !streamable(b.kind)
            || after.len() <= before.len()
            || !after.starts_with(before)
        {
            return None;
        }
        grown = Some((b.id.clone(), after[before.len()..].to_owned()));
    }
    grown
}

/// Blocks whose text grew (as an append) from `old` to `new`.
fn grown_blocks(old: &AgentSurfacePayload, new: &AgentSurfacePayload) -> BTreeSet<BlockId> {
    new.blocks
        .iter()
        .filter(|b| streamable(b.kind))
        .filter(|b| {
            let before = old.blocks.iter().find(|a| a.id == b.id).and_then(block_text);
            matches!((before, block_text(b)), (Some(x), Some(y)) if y.len() > x.len() && y.starts_with(x))
        })
        .map(|b| b.id.clone())
        .collect()
}

/// The patch operations that turn `old` into `new`, or `None` when the
/// blocks were reordered (a snapshot then).
pub(crate) fn patch_ops(
    old: &AgentSurfacePayload,
    new: &AgentSurfacePayload,
) -> Option<Vec<SurfacePatchOp>> {
    let mut ops = Vec::new();
    let new_ids: BTreeSet<&BlockId> = new.blocks.iter().map(|b| &b.id).collect();
    let mut kept: Vec<&Block> = Vec::new();
    for block in &old.blocks {
        if new_ids.contains(&block.id) {
            kept.push(block);
        } else {
            ops.push(SurfacePatchOp::RemoveBlock { block_id: block.id.clone() });
        }
    }
    let common: Vec<&BlockId> =
        new.blocks.iter().map(|b| &b.id).filter(|id| kept.iter().any(|k| &k.id == *id)).collect();
    if common.iter().copied().ne(kept.iter().map(|k| &k.id)) {
        return None;
    }
    for (index, block) in new.blocks.iter().enumerate() {
        match kept.iter().find(|k| k.id == block.id) {
            None => ops.push(SurfacePatchOp::InsertBlock { index, block: block.clone() }),
            Some(old) if *old != block => {
                ops.push(SurfacePatchOp::ReplaceBlock { block: block.clone() });
            }
            Some(_) => {}
        }
    }
    if old.actions != new.actions {
        ops.push(SurfacePatchOp::SetActions { actions: new.actions.clone() });
    }
    if old.fallback_text != new.fallback_text {
        ops.push(SurfacePatchOp::SetFallback { fallback_text: new.fallback_text.clone() });
    }
    Some(ops)
}

/// The daemon's view of one surface (`surface.get`).
struct View {
    reference: String,
    target: SurfaceTarget,
    version: u64,
    hash: SurfaceHash,
    closed: bool,
    out_of_sync: bool,
    renderer_error: Value,
    streams: Streams,
    posted_at: Option<Timestamp>,
    updated_at: Option<Timestamp>,
    snapshot: Value,
}

impl View {
    fn read(value: &Value) -> Option<Self> {
        let reference = text(value, "surface_ref");
        let version = value["version"].as_u64().unwrap_or(0);
        let hash = text(value, "surface_hash");
        // Version 0 is a placeholder for events held back before the
        // surface's creation arrived: nothing to show yet.
        if reference.is_empty() || version == 0 || hash.is_empty() || !value["snapshot"].is_object()
        {
            return None;
        }
        Some(Self {
            reference: reference.to_owned(),
            target: SurfaceTarget {
                conversation_id: text(value, "conversation_id").to_owned(),
                author_principal_id: text(value, "author_principal_id").to_owned(),
                surface_id: text(value, "surface_id").to_owned(),
            },
            version,
            hash: SurfaceHash::new(hash),
            closed: value["closed"].as_bool().unwrap_or(false),
            out_of_sync: value["out_of_sync"].as_bool().unwrap_or(false),
            renderer_error: value["renderer_error"].clone(),
            streams: streams_of(value),
            posted_at: seconds(&value["posted_at"]).or_else(|| seconds(&value["created_at"])),
            updated_at: seconds(&value["updated_at"]),
            snapshot: value["snapshot"].clone(),
        })
    }

    /// Whether `block`'s stream is open (appends permitted).
    fn open(&self, block: &BlockId) -> bool {
        !self.closed && self.streams.get(block.as_str()) == Some(&false)
    }

    /// Whether `block`'s stream is final.
    fn final_(&self, block: &BlockId) -> bool {
        self.closed || self.streams.get(block.as_str()) == Some(&true)
    }
}

impl Translator {
    /// The scope of a surface the source reported, by its desktop id.
    pub(crate) fn surface_target(&self, surface_ref: &str) -> Option<&SurfaceTarget> {
        self.surfaces.get(surface_ref).map(|s| &s.target)
    }

    /// The read that shows one context's surfaces.
    pub(crate) fn surfaces_read(context: &ContextId) -> Fetch {
        Fetch::Surfaces(context.clone(), None)
    }

    /// Notes the surface messages of a message list: when each surface was
    /// posted, and whether a surface not shown yet calls for a read.
    pub(super) fn surface_rows(&mut self, context: &ContextId, rows: &[&Value]) -> Vec<Fetch> {
        let mut unknown = false;
        for row in rows {
            let id = text(row, "surface_id");
            let sender = text(row, "sender_id");
            let author = self.directory.principal_of_device(sender).map(str::to_owned);
            let author = if self.self_device.as_deref() == Some(sender) {
                self.directory.self_principal.clone()
            } else {
                author
            };
            if let (false, Some(at)) = (id.is_empty(), seconds(&row["created_at"])) {
                let key = (context.clone(), id.to_owned(), author.clone().unwrap_or_default());
                let first = self.surface_times.entry(key).or_insert(at);
                *first = (*first).min(at);
            }
            unknown |= !self.surfaces.values().any(|s| {
                &s.context == context
                    && s.target.surface_id == id
                    && author.as_deref().is_none_or(|a| a == s.target.author_principal_id)
            });
        }
        if unknown { vec![Self::surfaces_read(context)] } else { Vec::new() }
    }

    /// A page of `surface.list`: the surfaces whose metadata differs from
    /// what was reported are read, and the next page.
    pub(super) fn surface_list(&self, context: &ContextId, value: &Value) -> Vec<Fetch> {
        let mut reads: Vec<Fetch> = list(value, "surfaces")
            .iter()
            .filter(|row| self.surface_stale(row))
            .map(|row| Fetch::Surface(text(row, "surface_ref").to_owned()))
            .collect();
        if let Some(cursor) = value["next_cursor"].as_str().filter(|c| !c.is_empty()) {
            reads.push(Fetch::Surfaces(context.clone(), Some(cursor.to_owned())));
        }
        reads
    }

    /// Whether surface metadata (a list row or a `surface.changed` body)
    /// names a state other than the one reported. A removal names a state
    /// other than any surface shown, and nothing for one never shown.
    pub(crate) fn surface_stale(&self, meta: &Value) -> bool {
        let reference = text(meta, "surface_ref");
        if reference.is_empty() {
            return false;
        }
        let known = self.surfaces.get(reference);
        if meta["removed"] == Value::Bool(true) {
            return known.is_some();
        }
        if self.surface_decision_waits(reference) {
            // An action answering a task step waits for its receipt.
            return true;
        }
        known.is_none_or(|known| {
            meta["version"].as_u64() != Some(known.version)
                || text(meta, "surface_hash") != known.hash.as_str()
                || meta["closed"].as_bool().unwrap_or(false) != known.closed
                || meta["out_of_sync"].as_bool().unwrap_or(false) != known.out_of_sync
                || meta["renderer_error"] != known.renderer_error
                || (meta.get("streams").is_some() && streams_of(meta) != known.streams)
        })
    }

    /// One surface read (`surface.get` of `reference`) as the events that
    /// report what changed.
    pub(super) fn surface_view(&mut self, reference: &str, value: &Value) -> Vec<DomainEvent> {
        if value.is_null() {
            // The daemon no longer holds it.
            return match self.surfaces.remove(reference) {
                Some(_) => vec![DomainEvent::Surface(SurfaceEvent::Removed {
                    surface_id: SurfaceId::new(reference),
                })],
                None => Vec::new(),
            };
        }
        self.surface_receipts(reference, value);
        let Some(view) = View::read(value) else { return Vec::new() };
        let context = ContextId::new(&view.target.conversation_id);
        if !self.contexts.contains_key(&context) {
            return Vec::new();
        }
        let surface = SurfaceId::new(&view.reference);
        let mut payload = catalog::payload_of(&view.snapshot, surface.clone());
        if payload.fallback_text.trim().is_empty() {
            // The protocol requires a fallback; never show an empty card.
            payload.fallback_text = view.target.surface_id.clone();
        }
        let known = self.surfaces.get(&view.reference).cloned();
        let (event, streaming) = match &known {
            Some(known) if view.version < known.version || known.closed => return Vec::new(),
            Some(known) => self.surface_change(known, &view, &payload),
            None => {
                // Created to be streamed into: its open blocks still empty.
                let writing: BTreeSet<BlockId> = if view.version == 1 {
                    payload
                        .blocks
                        .iter()
                        .filter(|b| {
                            streamable(b.kind) && block_text(b) == Some("") && view.open(&b.id)
                        })
                        .map(|b| b.id.clone())
                        .collect()
                } else {
                    BTreeSet::new()
                };
                let at = view.posted_at.or_else(|| {
                    self.surface_times.iter().find_map(|((c, id, author), at)| {
                        (c == &context
                            && id == &view.target.surface_id
                            && author == &view.target.author_principal_id)
                            .then_some(*at)
                    })
                });
                (Some(self.snapshot(&view, &context, &payload, &writing, at)), writing)
            }
        };
        let mut events: Vec<DomainEvent> = event.into_iter().map(DomainEvent::Surface).collect();
        let dated = known.as_ref().and_then(|k| k.updated_at) != view.updated_at;
        if let (Some(at), true) = (view.updated_at, dated || !events.is_empty()) {
            events.push(DomainEvent::Surface(SurfaceEvent::UpdatedAt { surface_id: surface, at }));
        }
        if events.is_empty() {
            return events;
        }
        self.surfaces.insert(
            view.reference.clone(),
            KnownSurface {
                target: view.target,
                context,
                version: view.version,
                hash: view.hash,
                payload,
                closed: view.closed,
                out_of_sync: view.out_of_sync,
                renderer_error: view.renderer_error,
                streams: view.streams,
                updated_at: view.updated_at,
                streaming,
            },
        );
        events
    }

    fn snapshot(
        &self,
        view: &View,
        context: &ContextId,
        payload: &AgentSurfacePayload,
        writing: &BTreeSet<BlockId>,
        at: Option<Timestamp>,
    ) -> SurfaceEvent {
        SurfaceEvent::Snapshot(Box::new(SurfaceSnapshot {
            moment_id: surface_moment_id(context, &view.reference),
            context_id: context.clone(),
            origin: ParticipantId::new(&view.target.author_principal_id),
            at,
            surface_id: payload.surface_id.clone(),
            version: view.version,
            hash: view.hash.clone(),
            blocks: payload.blocks.clone(),
            actions: payload.actions.clone(),
            fallback_text: payload.fallback_text.clone(),
            closed: view.closed,
            out_of_sync: view.out_of_sync,
            open_streams: writing.iter().cloned().collect(),
            final_streams: payload
                .blocks
                .iter()
                .filter(|b| streamable(b.kind) && view.final_(&b.id))
                .map(|b| b.id.clone())
                .collect(),
        }))
    }

    /// The event for a known surface's new state, with the blocks being
    /// written after it.
    fn surface_change(
        &self,
        known: &KnownSurface,
        view: &View,
        payload: &AgentSurfacePayload,
    ) -> (Option<SurfaceEvent>, BTreeSet<BlockId>) {
        let surface = payload.surface_id.clone();
        let same_flags = view.out_of_sync == known.out_of_sync;
        // Written: what was, while the daemon keeps it open; an append adds
        // its block, and so does growth across skipped versions (coalesced
        // appends). A patch that rewrites a text longer is not an append.
        // A stream reported final stays final (never reopened by a later
        // read), and once the author has ended a stream, the blocks it
        // created empty and has not written into are not being written.
        let any_final = payload.blocks.iter().any(|b| streamable(b.kind) && view.final_(&b.id));
        let writes = |b: &BlockId| {
            view.open(b)
                && known.streams.get(b.as_str()) != Some(&true)
                && !(any_final
                    && payload.blocks.iter().any(|x| &x.id == b && block_text(x) == Some("")))
        };
        let kept: BTreeSet<BlockId> =
            known.streaming.iter().filter(|b| writes(b)).cloned().collect();
        let with = |extra: &BTreeSet<BlockId>| -> BTreeSet<BlockId> {
            kept.iter().chain(extra.iter().filter(|b| writes(b))).cloned().collect()
        };
        let writing = if view.version > known.version + 1 {
            with(&grown_blocks(&known.payload, payload))
        } else {
            kept.clone()
        };
        if view.version == known.version {
            if view.hash == known.hash
                && same_flags
                && view.closed == known.closed
                && view.renderer_error == known.renderer_error
                && view.streams == known.streams
            {
                return (None, known.streaming.clone());
            }
            // Only the daemon's metadata changed (or it re-reported the
            // same version): the whole state again.
            let event = self.snapshot(view, &known.context, payload, &writing, None);
            return (Some(event), writing);
        }
        let next = view.version == known.version + 1 && same_flags && !known.out_of_sync;
        let content_same = known.payload == *payload;
        if next && view.closed && content_same {
            let close = SurfaceEvent::Close { surface_id: surface, version: view.version };
            return (Some(close), BTreeSet::new());
        }
        // Blocks whose stream ends with this version.
        let ending: Vec<&BlockId> = known.streaming.iter().filter(|b| view.final_(b)).collect();
        if next && !view.closed {
            if let Some((block, delta)) = appended(&known.payload, payload)
                && ending.iter().all(|b| **b == block)
            {
                let writing = with(&BTreeSet::from([block.clone()]));
                let event = SurfaceEvent::StreamAppend {
                    surface_id: surface,
                    version: view.version,
                    block_id: block.clone(),
                    text_delta: delta,
                    is_final: view.final_(&block),
                    hash: Some(view.hash.clone()),
                };
                return (Some(event), writing);
            }
            if content_same && let [block] = ending.as_slice() {
                // The version only ends a stream: its final, empty append.
                let event = SurfaceEvent::StreamAppend {
                    surface_id: surface,
                    version: view.version,
                    block_id: (*block).clone(),
                    text_delta: String::new(),
                    is_final: true,
                    hash: Some(view.hash.clone()),
                };
                return (Some(event), writing);
            }
            if ending.is_empty()
                && !content_same
                && let Some(ops) = patch_ops(&known.payload, payload)
                && !touches(&ops, &known.streaming)
            {
                let event = SurfaceEvent::Patch {
                    surface_id: surface,
                    version: view.version,
                    base_hash: known.hash.clone(),
                    ops,
                    hash: Some(view.hash.clone()),
                };
                return (Some(event), writing);
            }
        }
        // Anything else: the whole state.
        let event = self.snapshot(view, &known.context, payload, &writing, None);
        (Some(event), writing)
    }
}

/// Whether `ops` replace or remove a block being written (a patch then
/// cannot keep its stream; the whole state can).
fn touches(ops: &[SurfacePatchOp], writing: &BTreeSet<BlockId>) -> bool {
    ops.iter().any(|op| match op {
        SurfacePatchOp::ReplaceBlock { block } => writing.contains(&block.id),
        SurfacePatchOp::RemoveBlock { block_id } => writing.contains(block_id),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use ramflux_desktop_core::{Block, BlockKind, SurfaceAction};
    use serde_json::json;

    use super::*;

    fn payload(blocks: Vec<Block>) -> AgentSurfacePayload {
        AgentSurfacePayload {
            surface_id: SurfaceId::new("r"),
            fallback_text: "f".into(),
            blocks,
            actions: Vec::new(),
            streaming: false,
        }
    }

    fn text(id: &str, t: &str) -> Block {
        Block::new(id, BlockKind::Text, json!({ "text": t }))
    }

    #[test]
    fn an_append_is_growth_of_one_text_block_only() {
        let old = payload(vec![text("a", "Hel"), text("b", "")]);
        let new = payload(vec![text("a", "Hello"), text("b", "")]);
        assert_eq!(appended(&old, &new), Some((BlockId::new("a"), "lo".into())));
        let both = payload(vec![text("a", "Hello"), text("b", "x")]);
        assert_eq!(appended(&old, &both), None);
        let rewritten = payload(vec![text("a", "Hex"), text("b", "")]);
        assert_eq!(appended(&old, &rewritten), None);
        let mut fallback = new.clone();
        fallback.fallback_text = "g".into();
        assert_eq!(appended(&old, &fallback), None);
        let heading =
            payload(vec![Block::new("a", BlockKind::Heading, json!({ "text": "Hello" }))]);
        let heading_old =
            payload(vec![Block::new("a", BlockKind::Heading, json!({ "text": "Hel" }))]);
        assert_eq!(appended(&heading_old, &heading), None, "a heading does not stream");
    }

    #[test]
    fn patch_ops_remove_insert_replace_and_reset() {
        let old = payload(vec![text("a", "1"), text("b", "2"), text("c", "3")]);
        let mut new = payload(vec![text("a", "1"), text("n", "new"), text("c", "33")]);
        new.actions = vec![SurfaceAction {
            id: "go".into(),
            label: "Go".into(),
            risk: ramflux_desktop_core::Risk::Low,
            disabled_reason: None,
        }];
        new.fallback_text = "g".into();
        let ops = patch_ops(&old, &new).expect("ops");
        assert_eq!(
            ops,
            [
                SurfacePatchOp::RemoveBlock { block_id: BlockId::new("b") },
                SurfacePatchOp::InsertBlock { index: 1, block: text("n", "new") },
                SurfacePatchOp::ReplaceBlock { block: text("c", "33") },
                SurfacePatchOp::SetActions { actions: new.actions.clone() },
                SurfacePatchOp::SetFallback { fallback_text: "g".into() },
            ]
        );
        let reordered = payload(vec![text("c", "3"), text("a", "1")]);
        assert_eq!(patch_ops(&old, &reordered), None);
    }

    #[test]
    fn carrier_rows_are_surface_rows() {
        let bytes: Vec<Value> = b"ramflux.surface.v2\n{}".iter().map(|b| json!(b)).collect();
        assert!(is_surface_row(&json!({ "encrypted_body": bytes })));
        assert!(is_surface_row(&json!({ "content_type": "surface" })));
        assert!(!is_surface_row(&json!({ "encrypted_body": [104, 105] })));
    }
}
