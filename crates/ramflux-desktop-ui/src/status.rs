// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The design's status chips (`Status / Neutral` on board 00B, the tone
//! chips of G08 and F17) and tip bars (`RF / Tip bar` default, success,
//! warning and danger on board 00D).
//!
//! | | Chip | Tip bar |
//! |---|---|---|
//! | Size | 12 px text on its 17 px line, 8 px inset, 4 px above and below (25 px) | 34 px high, 12 px inset, `rf-radius-sm` |
//! | Corner | `rf-radius-chip` | `rf-radius-sm` |
//! | Neutral / default | `rf-surface`, `rf-text` | `rf-surface`, icon `rf-secondary` |
//! | Success | `rf-success-bg`, `rf-success` | `rf-success-bg`, icon `rf-success` |
//! | Warning | `rf-warning-bg`, `rf-warning` | `rf-warning-bg`, icon `rf-warning` |
//! | Danger | `rf-danger-bg`, `rf-danger` | `rf-danger-bg`, icon `rf-danger` |
//!
//! A tip bar's text is 13 px `rf-text` after a 16 px icon and an 8 px gap;
//! it wraps when it is longer than the bar.

use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, Hsla, SharedString, div};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{design_px, space, style};

use crate::assets::DesignType as _;

/// The tone of a chip or tip bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Success,
    Warning,
    Danger,
}

/// Height of a single-line tip bar.
const TIP_HEIGHT_PX: f32 = 34.0;
/// The tip bar's icon.
const TIP_ICON_PX: f32 = 16.0;
/// A chip's inset above and below its line.
const CHIP_INSET_PX: f32 = 4.0;

/// Fill and foreground of a tone.
fn colours(tone: Tone, cx: &App) -> (Hsla, Hsla) {
    let p = ramflux_desktop_navigation::palette(cx);
    match tone {
        Tone::Neutral => (p.surface, p.text),
        Tone::Success => (p.success_bg, p.success),
        Tone::Warning => (p.warning_bg, p.warning),
        Tone::Danger => (p.danger_bg, p.danger),
    }
}

/// A status chip.
pub fn status_chip(tone: Tone, text: impl Into<SharedString>, cx: &App) -> Div {
    let p = ramflux_desktop_navigation::palette(cx);
    let (fill, ink) = colours(tone, cx);
    div()
        .flex_none()
        .px(space(2.))
        .py(design_px(CHIP_INSET_PX))
        .rounded(design_px(p.radius_chip))
        .bg(fill)
        .text_color(ink)
        .design_type(style::meta())
        .child(text.into())
}

/// The icon a tip bar of `tone` shows.
pub fn tip_icon(tone: Tone) -> IconName {
    match tone {
        Tone::Neutral => IconName::Info,
        Tone::Success => IconName::CircleCheck,
        Tone::Warning => IconName::TriangleAlert,
        Tone::Danger => IconName::CircleAlert,
    }
}

/// A tip bar with `tone`'s icon (or `icon`) before `text`.
pub fn tip_bar(tone: Tone, icon: Option<IconName>, text: impl Into<SharedString>, cx: &App) -> Div {
    let p = ramflux_desktop_navigation::palette(cx);
    let (fill, accent) = colours(tone, cx);
    let icon_colour = if tone == Tone::Neutral { p.secondary } else { accent };
    h_flex()
        .w_full()
        .min_w_0()
        .min_h(design_px(TIP_HEIGHT_PX))
        .items_center()
        .gap(space(2.))
        .px(space(3.))
        .py(space(1.5))
        .rounded(design_px(p.radius_sm))
        .bg(fill)
        .child(
            Icon::new(icon.unwrap_or_else(|| tip_icon(tone)))
                .flex_none()
                .size(design_px(TIP_ICON_PX))
                .text_color(icon_colour),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .whitespace_normal()
                .text_color(p.text)
                .design_type(style::secondary())
                .child(text.into()),
        )
}
