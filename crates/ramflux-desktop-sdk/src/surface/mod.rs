// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Agent surfaces on the daemon: what the desktop reads and sends.
//!
//! The daemon applies every surface event, keeps the current state per
//! surface scoped by conversation, author and surface id (`surface_ref`),
//! and computes the protocol's snapshot hash. The desktop never hashes or
//! re-validates a surface: it shows the daemon's state.
//!
//! Surfaces are cards (`urn:ramflux:catalog:rf-card:1.0`). Their components
//! are drawn by the card runtime of the daemon (`card.session.*`); the
//! surface's own fallback text is shown where the runtime does not draw it.

/// The daemon's scoped reference (`surface_ref`, the desktop's surface
/// id) of the author's surface `surface_id` in `conversation`; `None` when
/// the ids are not canonical JSON strings.
pub fn surface_ref(conversation: &str, author: &str, surface_id: &str) -> Option<String> {
    let bytes = ramflux_protocol::canonical_json_bytes(&(conversation, author, surface_id)).ok()?;
    Some(ramflux_protocol::hash_base64url("ramflux.surface.identity.v1", &bytes))
}

/// Where a surface lives, as the daemon scopes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceTarget {
    /// The conversation.
    pub conversation_id: String,
    /// The author's principal.
    pub author_principal_id: String,
    /// The author's surface id (unique per author and conversation only).
    pub surface_id: String,
}
