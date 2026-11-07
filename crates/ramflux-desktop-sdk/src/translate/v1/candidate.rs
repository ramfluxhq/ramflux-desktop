// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The early test seam: typed updates through the v1 projection, without
//! the bus decode path (`test-support` only; production never builds it).

use ramflux_protocol::message::MessageUpdate;

use super::{Translator, project};
use crate::translate::Translated;

/// Projects `updates` in order against `translator`'s state.
pub fn apply(translator: &mut Translator, updates: &[MessageUpdate]) -> Translated {
    let events = updates.iter().flat_map(|update| project(translator, update)).collect();
    Translated::events(events)
}

/// An object fetch failed: the media rows of `object_id` follow the typed
/// refusal (revoked) or the generic failure (unavailable).
pub fn media_failed(
    translator: &mut Translator,
    object_id: &str,
    failure: &crate::bus::BusError,
) -> Translated {
    Translated::events(super::media::refuse(translator, object_id, failure))
}

/// How many v1 surfaces and pending surface actions `translator` retains.
#[must_use]
pub fn surface_counts(translator: &Translator) -> (usize, usize) {
    (translator.v1_surfaces.surfaces(), translator.v1_surfaces.pending())
}
