// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Element ids derived from Ramflux ids, never from list positions.

use std::fmt::Display;

use gpui_kit::ElementId;

/// `"{prefix}-{id}"` as an element id, e.g. `moment-m-approve`.
pub fn domain_id(prefix: &str, id: &impl Display) -> ElementId {
    ElementId::from(format!("{prefix}-{id}"))
}
