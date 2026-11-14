// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The settings column's building blocks as the design draws them (G01–G13):
//! a section under a semibold heading closed by a rule, a two-line row with
//! an optional trailing chip or control, and a labelled field.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, ElementId, IntoElement, SharedString, div, rems};

fn rem(px: f32) -> gpui_kit::Rems {
    rems(px / 15.0)
}

/// A section: its heading, then `children`, closed by a rule.
pub(crate) fn section(
    id: impl Into<ElementId>,
    title: SharedString,
    cx: &App,
) -> impl ParentElement + IntoElement {
    let palette = ramflux_desktop_navigation::palette(cx);
    v_flex()
        .id(id)
        .test_support()
        .w_full()
        .gap_3()
        .pb(rem(20.0))
        .border_b_1()
        .border_color(palette.line)
        .child(div().text_size(rem(palette.size_subheading)).font_semibold().child(title))
}

/// A two-line row (title over detail) with a trailing element, on a rule.
pub(crate) fn row(
    title: SharedString,
    detail: SharedString,
    trailing: Option<AnyElement>,
    cx: &App,
) -> impl IntoElement {
    let palette = ramflux_desktop_navigation::palette(cx);
    h_flex()
        .w_full()
        .gap_4()
        .py_3()
        .border_b_1()
        .border_color(palette.line)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().text_size(rem(palette.size_body)).font_medium().child(title))
                .child(
                    div()
                        .text_size(rem(palette.size_secondary))
                        .text_color(palette.secondary)
                        .child(detail),
                ),
        )
        .children(trailing)
}

/// A small status chip ("Current", "Default").
pub(crate) fn chip(text: SharedString, cx: &App) -> AnyElement {
    let palette = ramflux_desktop_navigation::palette(cx);
    div()
        .px_2()
        .py(rem(4.0))
        .rounded(rem(palette.radius_chip))
        .bg(palette.surface)
        .text_size(rem(palette.size_meta))
        .child(text)
        .into_any_element()
}

/// One line of secondary text.
pub(crate) fn help(text: SharedString, cx: &App) -> impl IntoElement {
    let palette = ramflux_desktop_navigation::palette(cx);
    div()
        .text_size(rem(palette.size_secondary))
        .text_color(palette.secondary)
        .whitespace_normal()
        .child(text)
}
