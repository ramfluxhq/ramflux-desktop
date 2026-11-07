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
