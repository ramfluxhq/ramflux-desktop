// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The design's identity avatars (`Avatar / Identity`, `Agent`, `Group` on
//! boards 00B and 00D): the catalog avatar frame with the initial in
//! semibold, which the generated frame leaves to the inherited weight.

use gpui_kit::component::StyledExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{Div, div};
use ramflux_desktop_navigation::generated::catalog::{self, AvatarKind};
use ramflux_desktop_navigation::generated::rf_tokens::RFTheme;

/// An avatar of `kind` showing `initial`.
pub fn avatar(initial: gpui_kit::SharedString, kind: AvatarKind, theme: RFTheme) -> Div {
    // Light ink on the agent's dark fill reads heavier than dark on light,
    // so an agent's initial is one weight lighter (measured against 00B).
    let frame = div().flex_none();
    let frame =
        if matches!(kind, AvatarKind::Agent) { frame.font_medium() } else { frame.font_semibold() };
    frame.child(catalog::avatar(initial, kind, theme))
}
