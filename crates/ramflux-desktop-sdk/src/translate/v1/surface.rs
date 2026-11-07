// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A v1 surface (content kind 9): the catalog gate and answer binding.
//!
//! - **Catalog gate.** A created surface is drawn natively only when its
//!   `(catalog, version)` is in the native registry (`basic/1`). The
//!   declarative web catalog (`rf-web/v1`) is known but has no renderer until
//!   its activation merges (its arm of [`route`] changes then); an unknown
//!   catalog and a known catalog at an unknown version are not drawn either. Each of these shows the author's
//!   fallback text only, with no blocks and no actions, and stays that way.
//! - **Answer binding.** An action is remembered with the principal that
//!   requested it. A receipt holds the original action carrier (one level,
//!   action only) and is accepted only for the requester recorded for that
//!   action; any other requester makes the surface inert. A receipt for an
//!   action never recorded changes nothing.
//! - **Bounds.** At most [`MAX_SURFACES`] surfaces and [`MAX_PENDING`]
//!   pending actions are retained; everything is dropped on lock. A replay
//!   at or below the known version is ignored and an inert surface is never
//!   revived.
//!
//! Update, patch, stream and delete commands of an existing surface follow
//! the live path (P16); here only creation, actions and receipts project.

use std::collections::BTreeMap;

use ramflux_desktop_core::{
    Block, ContextId, DomainEvent, ParticipantId, SurfaceAction, SurfaceEvent, SurfaceHash,
    SurfaceId, SurfaceSnapshot, Timestamp,
};
use ramflux_protocol::message::{
    CreateSurfaceCommand, EventIdentity, JsonValue, Surface, SurfaceCarrier, json_value,
    surface::Payload as SurfacePayload, surface_carrier::Payload as CarrierPayload,
    surface_command::Value as Command,
};
use serde_json::{Map, Number, Value};

use super::context_of;
use crate::surface::catalog;
use crate::translate::Translator;
use crate::translate::surfaces::surface_moment_id;

/// The declarative web catalog name and version.
const WEB_CATALOG: (&str, &str) = ("rf-web/v1", "1");

/// Surfaces retained per account.
const MAX_SURFACES: usize = 512;

/// Requested actions retained until their receipt.
const MAX_PENDING: usize = 256;

/// Deepest JSON nesting read from a component.
const MAX_DEPTH: usize = 16;

/// Components read from one surface.
const MAX_COMPONENTS: usize = 512;

/// How a catalog is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Drawn by the native `basic/1` block registry.
    Native,
    /// Not drawn: fallback text only.
    Inert(Inert),
}

/// Why a surface is not drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inert {
    /// The web catalog is known, its renderer is not active.
    WebInactive,
    /// A catalog this build does not know.
    UnknownCatalog,
    /// A known catalog at a version this build does not know.
    UnknownVersion,
    /// The components or actions cannot be read.
    Malformed,
    /// An action or receipt failed its requester binding.
    Unbound,
}

/// Routes `(catalog, version)` to its renderer.
#[must_use]
pub fn route(catalog_name: &str, version: &str) -> Route {
    if (catalog_name, version) == (catalog::CATALOG, catalog::CATALOG_VERSION) {
        return Route::Native;
    }
    if catalog_name == WEB_CATALOG.0 {
        return Route::Inert(if version == WEB_CATALOG.1 {
            Inert::WebInactive
        } else {
            Inert::UnknownVersion
        });
    }
    if catalog_name == catalog::CATALOG {
        return Route::Inert(Inert::UnknownVersion);
    }
    Route::Inert(Inert::UnknownCatalog)
}

/// What is kept of one surface.
#[derive(Debug, Clone)]
struct Known {
    context: ContextId,
    origin: ParticipantId,
    at: Timestamp,
    version: u64,
    hash: SurfaceHash,
    fallback_text: String,
    inert: bool,
}

/// The v1 surfaces seen and the actions waiting for a receipt.
#[derive(Debug, Default)]
pub struct Book {
    surfaces: BTreeMap<String, Known>,
    /// `(surface reference, action message id)` and the requesting
    /// principal, oldest first.
    pending: Vec<((String, String), String)>,
}

impl Book {
    /// Forgets everything (lock, account switch).
    pub fn clear(&mut self) {
        self.surfaces.clear();
        self.pending.clear();
    }

    /// Surfaces retained.
    #[must_use]
    pub fn surfaces(&self) -> usize {
        self.surfaces.len()
    }

    /// Actions waiting for their receipt.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.pending.len()
    }
}

/// The desktop id of a surface: its author and its own id (two authors may
/// use one id in a conversation).
fn reference(carrier: &SurfaceCarrier, surface_id: &str) -> String {
    format!("{}:{surface_id}", carrier.author_principal_id)
}

pub(super) fn project(
    translator: &mut Translator,
    identity: &EventIdentity,
    surface: &Surface,
) -> Vec<DomainEvent> {
    let Some(SurfacePayload::Delivery(delivery)) = &surface.payload else { return Vec::new() };
    let Some(carrier) = &delivery.carrier else { return Vec::new() };
    // The envelope and the carrier must name one conversation.
    if carrier.conversation_id != identity.conversation_id || carrier.author_principal_id.is_empty()
    {
        return Vec::new();
    }
    let context = context_of(identity);
    match &carrier.payload {
        Some(CarrierPayload::Event(event)) if event.conversation_id == carrier.conversation_id => {
            match event.command.as_ref().and_then(|c| c.value.as_ref()) {
                Some(Command::CreateSurface(create)) => create
                    .value
                    .as_ref()
                    .filter(|value| value.surface_id == carrier.surface_id)
                    .map(|value| created(translator, &context, carrier, event.version, value))
                    .unwrap_or_default(),
                // Existing surfaces: the live path (P16). An inert surface
                // is never revived, so nothing here can reach it.
                _ => Vec::new(),
            }
        }
        Some(CarrierPayload::Action(action)) if action.surface_id == carrier.surface_id => {
            requested(translator, carrier, action.version)
        }
        Some(CarrierPayload::ActionReceipt(original)) => answered(translator, carrier, original),
        _ => Vec::new(),
    }
}

fn created(
    translator: &mut Translator,
    context: &ContextId,
    carrier: &SurfaceCarrier,
    version: u64,
    create: &CreateSurfaceCommand,
) -> Vec<DomainEvent> {
    let key = reference(carrier, &create.surface_id);
    if version == 0 {
        return Vec::new();
    }
    let book = &mut translator.v1_surfaces;
    match book.surfaces.get(&key) {
        // A replay never rolls back, and an inert surface never revives.
        Some(known) if version <= known.version || known.inert => return Vec::new(),
        None if book.surfaces.len() >= MAX_SURFACES => return Vec::new(),
        _ => {}
    }
    let fallback = if create.fallback_text.trim().is_empty() {
        create.surface_id.clone()
    } else {
        create.fallback_text.clone()
    };
    let (route, content) = match route(&create.catalog, &create.catalog_version) {
        Route::Native => match content_of(create) {
            Some(content) => (Route::Native, content),
            None => (Route::Inert(Inert::Malformed), (Vec::new(), Vec::new())),
        },
        inert => (inert, (Vec::new(), Vec::new())),
    };
    let hash = carrier
        .snapshot
        .as_ref()
        .and_then(|proof| proof.surface_hash.clone())
        .filter(|hash| !hash.is_empty())
        .map_or_else(|| SurfaceHash::of_version(version), SurfaceHash::new);
    let at = carrier.posted_at.map(Timestamp);
    let known = Known {
        context: context.clone(),
        origin: ParticipantId::new(&carrier.author_principal_id),
        at: at.unwrap_or(Timestamp(0)),
        version,
        hash,
        fallback_text: fallback,
        inert: matches!(route, Route::Inert(_)),
    };
    let event = snapshot(&key, &known, content);
    translator.v1_surfaces.surfaces.insert(key, known);
    vec![DomainEvent::Surface(event)]
}

/// An action was requested: remember who asked for it.
fn requested(
    translator: &mut Translator,
    carrier: &SurfaceCarrier,
    version: u64,
) -> Vec<DomainEvent> {
    let key = reference(carrier, &carrier.surface_id);
    let requester = carrier.sender_principal_id.clone();
    let book = &mut translator.v1_surfaces;
    let Some(known) = book.surfaces.get(&key) else { return Vec::new() };
    if known.inert || version != known.version || requester.is_empty() {
        return Vec::new();
    }
    let slot = (key.clone(), carrier.message_id.clone());
    match book.pending.iter().find(|(pending, _)| *pending == slot) {
        Some((_, first)) if *first == requester => return Vec::new(),
        // The same action named by a second requester: it cannot be
        // answered for either.
        Some(_) => return neutralize(translator, &key),
        None => {}
    }
    if book.pending.len() >= MAX_PENDING {
        book.pending.remove(0);
    }
    book.pending.push((slot, requester));
    Vec::new()
}

/// A receipt answers the original action carrier it holds.
fn answered(
    translator: &mut Translator,
    receipt: &SurfaceCarrier,
    original: &SurfaceCarrier,
) -> Vec<DomainEvent> {
    let key = reference(receipt, &receipt.surface_id);
    if !translator.v1_surfaces.surfaces.contains_key(&key) {
        return Vec::new();
    }
    // One level, action only, same surface and author as the receipt.
    let bound = matches!(&original.payload, Some(CarrierPayload::Action(action))
            if action.surface_id == receipt.surface_id)
        && original.surface_id == receipt.surface_id
        && original.author_principal_id == receipt.author_principal_id
        && original.conversation_id == receipt.conversation_id;
    if !bound {
        return neutralize(translator, &key);
    }
    let book = &mut translator.v1_surfaces;
    let slot = (key.clone(), original.message_id.clone());
    let Some(index) = book.pending.iter().position(|(pending, _)| *pending == slot) else {
        // Never requested here: nothing to bind it to, nothing to change.
        return Vec::new();
    };
    if book.pending[index].1 != original.sender_principal_id {
        return neutralize(translator, &key);
    }
    book.pending.remove(index);
    Vec::new()
}

/// Makes a surface inert: fallback text only, no blocks, no actions.
fn neutralize(translator: &mut Translator, key: &str) -> Vec<DomainEvent> {
    let book = &mut translator.v1_surfaces;
    book.pending.retain(|((surface, _), _)| surface != key);
    let Some(known) = book.surfaces.get_mut(key) else { return Vec::new() };
    if known.inert {
        return Vec::new();
    }
    known.inert = true;
    let event = snapshot(key, known, (Vec::new(), Vec::new()));
    vec![DomainEvent::Surface(event)]
}

fn snapshot(key: &str, known: &Known, content: (Vec<Block>, Vec<SurfaceAction>)) -> SurfaceEvent {
    let (blocks, actions) = content;
    SurfaceEvent::Snapshot(Box::new(SurfaceSnapshot {
        moment_id: surface_moment_id(&known.context, key),
        context_id: known.context.clone(),
        origin: known.origin.clone(),
        at: (known.at != Timestamp(0)).then_some(known.at),
        surface_id: SurfaceId::new(key),
        version: known.version,
        hash: known.hash.clone(),
        blocks,
        actions,
        fallback_text: known.fallback_text.clone(),
        closed: false,
        out_of_sync: false,
        open_streams: Vec::new(),
        final_streams: Vec::new(),
    }))
}

/// The blocks and actions of a `basic/1` surface, or `None` when they
/// cannot be read (a component without an id, a repeated id, too many,
/// nested too deep or a number that is not one).
fn content_of(create: &CreateSurfaceCommand) -> Option<(Vec<Block>, Vec<SurfaceAction>)> {
    if create.components.len() > MAX_COMPONENTS {
        return None;
    }
    let mut blocks: Vec<Block> = Vec::with_capacity(create.components.len());
    for component in &create.components {
        let block = catalog::block_of(&json_of(component, 0)?)?;
        if blocks.iter().any(|known| known.id == block.id) {
            return None;
        }
        blocks.push(block);
    }
    let mut actions: Vec<SurfaceAction> = Vec::new();
    for allowed in &create.allowed_actions {
        let action = catalog::action_of(&json_of(allowed, 0)?)?;
        if actions.iter().any(|known| known.id == action.id) {
            return None;
        }
        actions.push(action);
    }
    Some((blocks, actions))
}

/// A typed protocol value as plain JSON.
fn json_of(value: &JsonValue, depth: usize) -> Option<Value> {
    if depth > MAX_DEPTH {
        return None;
    }
    Some(match value.value.as_ref()? {
        json_value::Value::Null(_) => Value::Null,
        json_value::Value::Boolean(flag) => Value::Bool(*flag),
        json_value::Value::Number(token) => Value::Number(token.parse::<Number>().ok()?),
        json_value::Value::Text(text) => Value::String(text.clone()),
        json_value::Value::Array(array) => Value::Array(
            array.values.iter().map(|v| json_of(v, depth + 1)).collect::<Option<Vec<_>>>()?,
        ),
        json_value::Value::Object(object) => {
            let mut map = Map::new();
            for member in &object.members {
                let inner = member.value.as_ref()?;
                if map.insert(member.key.clone(), json_of(inner, depth + 1)?).is_some() {
                    return None;
                }
            }
            Value::Object(map)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_by_catalog_and_version() {
        assert_eq!(route("basic", "1"), Route::Native);
        assert_eq!(route("rf-web/v1", "1"), Route::Inert(Inert::WebInactive));
        assert_eq!(route("rf-web/v1", "2"), Route::Inert(Inert::UnknownVersion));
        assert_eq!(route("basic", "2"), Route::Inert(Inert::UnknownVersion));
        assert_eq!(route("basic", ""), Route::Inert(Inert::UnknownVersion));
        assert_eq!(route("custom", "1"), Route::Inert(Inert::UnknownCatalog));
        assert_eq!(route("Basic", "1"), Route::Inert(Inert::UnknownCatalog));
    }
}
