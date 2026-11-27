// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The design's identity avatars (`Avatar / Identity`, `Agent`, `Group` on
//! boards 00B and 00D): the catalog avatar frame with the initial in
//! semibold, which the generated frame leaves to the inherited weight.
//!
//! Group avatars follow the boards: a named group shows its initial in the
//! person circle (boards 02 and C02, list row and group profile); a group with
//! no name to take an initial from shows the users glyph (board 00D,
//! `Avatar / Group`). The generated group style paints its ink in its own fill,
//! so a group is never drawn through it.

use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, StyledExt as _};
use gpui_kit::prelude::*;
use gpui_kit::{Div, div};
use ramflux_desktop_navigation::generated::catalog::{self, AvatarKind};
use ramflux_desktop_navigation::generated::rf_tokens::RFTheme;
use ramflux_desktop_navigation::scale::design_px;

/// An avatar of `kind` showing `initial`.
pub fn avatar(initial: gpui_kit::SharedString, kind: AvatarKind, theme: RFTheme) -> Div {
    // Light ink on the agent's dark fill reads heavier than dark on light,
    // so an agent's initial is one weight lighter (measured against 00B).
    let frame = div().flex_none();
    let frame =
        if matches!(kind, AvatarKind::Agent) { frame.font_medium() } else { frame.font_semibold() };
    match kind {
        AvatarKind::Group if initial.is_empty() => frame
            .size(design_px(40.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(theme.avatar_person)
            .text_color(theme.on_avatar_person)
            .child(Icon::new(IconName::Users).size(design_px(20.0))),
        AvatarKind::Group => frame.child(catalog::avatar(initial, AvatarKind::Person, theme)),
        kind => frame.child(catalog::avatar(initial, kind, theme)),
    }
}
