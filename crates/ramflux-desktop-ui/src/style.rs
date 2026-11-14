// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The design's button shapes on top of Kit's buttons (design components
//! `Button / Primary` and `Button / Secondary`): 32 px high, 12 px inset,
//! the button radius and the button text size; the secondary one is drawn
//! on `rf-surface-white` with an `rf-text` border.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{App, ElementId, px, rems};

/// The design's button geometry and colours for a Kit [`Button`].
pub trait DesignButton: Sized {
    /// The design's secondary button (outline on the white surface).
    fn design_secondary(self, cx: &App) -> Self;
    /// The design's primary button.
    fn design_primary(self, cx: &App) -> Self;
    /// The secondary button where no theme is at hand (a table cell): the
    /// geometry, with the outline in the theme's text colour once painted.
    fn design_secondary_compact(self) -> Self;
}

impl DesignButton for Button {
    fn design_secondary(self, cx: &App) -> Self {
        let palette = ramflux_desktop_navigation::palette(cx);
        self.outline()
            .h(rems(32.0 / 15.0))
            .px(rems(12.0 / 15.0))
            .rounded(px(palette.radius_button))
            .border_color(palette.text)
            .bg(palette.surface_white)
            .text_color(palette.text)
            .text_size(rems(palette.size_button / 15.0))
    }

    fn design_secondary_compact(self) -> Self {
        self.outline().h(rems(32.0 / 15.0)).px(rems(12.0 / 15.0))
    }

    fn design_primary(self, cx: &App) -> Self {
        let palette = ramflux_desktop_navigation::palette(cx);
        self.primary()
            .h(rems(32.0 / 15.0))
            .px(rems(12.0 / 15.0))
            .rounded(px(palette.radius_button))
            .text_size(rems(palette.size_button / 15.0))
    }
}

/// A design primary button.
pub fn primary_button(id: impl Into<ElementId>, cx: &App) -> Button {
    Button::new(id).design_primary(cx)
}

/// A design secondary button.
pub fn secondary_button(id: impl Into<ElementId>, cx: &App) -> Button {
    Button::new(id).design_secondary(cx)
}
