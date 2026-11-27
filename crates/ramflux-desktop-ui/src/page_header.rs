// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The design's page header (`RF / Page header` on board 00D): 88 px high,
//! 32 px inset either side, a 1 px rule below. The title is 17 px semibold
//! over a 13 px secondary description; an optional leading element (an
//! avatar) and a trailing action sit 12 px from the text. The generated
//! catalog header always adds empty leading and badge slots, each with its
//! 12 px gap, which pushed the title 12 px in from the board's.

use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, SharedString, div};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::generated::rf_tokens::RFTheme;
use ramflux_desktop_navigation::scale::{DesignText as _, TextStyle, design_px, style};

/// The header's design height.
const HEIGHT_PX: f32 = 88.0;

/// A page header with `title` set in `title_style` (17 px `style::subheading()`
/// for a section page, 24 px `style::page()` for Settings) and `description`
/// (omitted when empty).
pub fn page_header(
    title: impl Into<SharedString>,
    title_style: TextStyle,
    description: impl Into<SharedString>,
    leading: Option<AnyElement>,
    trailing: impl IntoElement,
    theme: RFTheme,
) -> impl IntoElement {
    let description: SharedString = description.into();
    h_flex()
        .w_full()
        .min_w_0()
        .h(design_px(HEIGHT_PX))
        .items_center()
        .gap(design_px(12.0))
        .px(design_px(32.0))
        .border_b_1()
        .border_color(theme.line)
        .children(leading)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(div().text_color(theme.text).design_text(title_style).child(title.into()))
                .when(!description.is_empty(), |column| {
                    column.child(
                        div()
                            .text_color(theme.secondary)
                            .design_text(style::secondary())
                            .child(description.clone()),
                    )
                }),
        )
        .child(trailing)
}
