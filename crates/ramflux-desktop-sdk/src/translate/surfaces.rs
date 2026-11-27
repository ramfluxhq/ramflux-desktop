// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Agent surfaces the daemon holds, as surface moments.
//!
//! The daemon reports that a surface changed (`surface.changed`, metadata
//! only) or lists a conversation's surfaces (`surface.list`); the source then
//! reads the surface (`surface.get`) and reports its state as a snapshot
//! when that state differs from the one last reported. A surface the daemon
//! no longer holds (`surface.changed` with `removed`, then a read that finds
//! nothing) is removed.
//!
//! A change of a surface's hash also tells the card runtime the held card
//! is stale: the card is read again (`card.session.update`), its tree is the
//! card's own content.
//!
//! Every snapshot carries the daemon's hash; surfaces are keyed by the
//! daemon's scoped `surface_ref`, which is also the desktop's surface id:
//! two authors may use the same surface id in one conversation.
//!
//! Times: the daemon's `posted_at` (the author's signed posting time) dates
//! the surface moment; for a surface without one, the time it was first
//! seen. `updated_at` is reported after each change. Both are the author's
//! clock, for display only.
//!
//! Surface rows of a conversation's read model are not messages (the
//! surface moment is).

use ramflux_desktop_core::{
    CardEvent, ContextId, DomainEvent, ParticipantId, SurfaceAction, SurfaceEvent, SurfaceHash,
    SurfaceId, SurfaceSnapshot, Timestamp,
};
use serde_json::Value;

use super::{Translator, list, seconds, text};
use crate::plan::fetch::Fetch;
use crate::surface::SurfaceTarget;

/// What the source last reported about one surface.
#[derive(Debug, Clone)]
pub(crate) struct KnownSurface {
    pub(crate) target: SurfaceTarget,
    version: u64,
    hash: SurfaceHash,
    closed: bool,
    out_of_sync: bool,
    /// Its last-updated time, as reported.
    updated_at: Option<Timestamp>,
}

/// The surface moment of `surface_ref` in `context`.
pub(crate) fn surface_moment_id(
    context: &ContextId,
    surface_ref: &str,
) -> ramflux_desktop_core::MomentId {
    ramflux_desktop_core::MomentId::new(format!("surface:{context}:{surface_ref}"))
}

/// The daemon's view of one surface (`surface.get`).
struct View {
    reference: String,
    target: SurfaceTarget,
    version: u64,
    hash: SurfaceHash,
    closed: bool,
    out_of_sync: bool,
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
            posted_at: seconds(&value["posted_at"]).or_else(|| seconds(&value["created_at"])),
            updated_at: seconds(&value["updated_at"]),
            snapshot: value["snapshot"].clone(),
        })
    }
}

/// The actions the snapshot offers, as the daemon lists them.
fn actions_of(snapshot: &Value) -> Vec<SurfaceAction> {
    snapshot
        .get("actions")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| serde_json::from_value::<SurfaceAction>(row.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// The fallback text of a snapshot, never empty (the protocol requires one).
fn fallback_of(snapshot: &Value, reference: &str) -> String {
    snapshot
        .get("fallbackText")
        .and_then(Value::as_str)
        .filter(|t| !t.trim().is_empty())
        .map_or_else(|| reference.to_owned(), str::to_owned)
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
        let known = self.surfaces.get(&view.reference).cloned();
        if let Some(known) = &known
            && view.version < known.version
        {
            return Vec::new();
        }
        let unchanged = known.as_ref().is_some_and(|k| {
            k.version == view.version
                && k.hash == view.hash
                && k.closed == view.closed
                && k.out_of_sync == view.out_of_sync
                && k.updated_at == view.updated_at
        });
        if unchanged {
            return Vec::new();
        }
        let mut events = Vec::new();
        let card_moved = known.as_ref().is_some_and(|k| k.hash != view.hash);
        let changed = known.as_ref().is_none_or(|k| {
            k.version != view.version
                || k.hash != view.hash
                || k.closed != view.closed
                || k.out_of_sync != view.out_of_sync
        });
        if changed {
            let at = if known.is_none() { view.posted_at } else { None };
            events.push(DomainEvent::Surface(SurfaceEvent::Snapshot(Box::new(SurfaceSnapshot {
                moment_id: surface_moment_id(&context, &view.reference),
                context_id: context.clone(),
                origin: ParticipantId::new(&view.target.author_principal_id),
                at,
                surface_id: surface.clone(),
                version: view.version,
                hash: view.hash.clone(),
                actions: actions_of(&view.snapshot),
                fallback_text: fallback_of(&view.snapshot, &view.reference),
                closed: view.closed,
                out_of_sync: view.out_of_sync,
                order: None,
            }))));
        }
        if card_moved {
            // The card is a view of this surface: the runtime reads it again.
            events.push(DomainEvent::Card(CardEvent::Stale { surface_id: surface.clone() }));
        }
        if let Some(at) = view.updated_at
            && known.as_ref().and_then(|k| k.updated_at) != Some(at)
        {
            events.push(DomainEvent::Surface(SurfaceEvent::UpdatedAt { surface_id: surface, at }));
        }
        self.surfaces.insert(
            view.reference.clone(),
            KnownSurface {
                target: view.target,
                version: view.version,
                hash: view.hash,
                closed: view.closed,
                out_of_sync: view.out_of_sync,
                updated_at: view.updated_at,
            },
        );
        events
    }
}
