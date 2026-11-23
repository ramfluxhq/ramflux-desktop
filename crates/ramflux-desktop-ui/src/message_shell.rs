// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The design's message bubble (`RF / Bubble / Peer`, `Own`, `Group` on
//! board 00B; boards 02 and H04).
//!
//! A bubble is its content on the incoming (`rf-incoming`, `rf-text`) or
//! outgoing (`rf-outgoing`, `rf-outgoing-text`) fill, 8 px above and below
//! and 12 px either side, at the bubble radius (`rf-radius-bubble`). One
//! line of message text (15 px on its 24 px line) makes a 40 px bubble.
//! The content arranges sender, body and time; the shell adds nothing else
//! (the generated catalog bubble it replaces put empty quote, attachment
//! and footer slots around the content, each with its 4 px gap, so every
//! bubble was 12 px taller than the boards).

use gpui_kit::prelude::*;
use gpui_kit::{App, Div, ElementId, Stateful, div};
use ramflux_desktop_navigation::scale::{DesignText as _, design_px, space, style};

/// Whose message a bubble holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BubbleKind {
    /// Someone else's message.
    Peer,
    /// The account's own message.
    Own,
    /// A peer message joined to the one above it (same author, minutes
    /// apart): no sender line.
    Joined,
}

/// The bubble's inset above and below its content.
const INSET_PX: f32 = 8.0;

/// A message bubble around `content`.
pub fn message_shell(
    id: impl Into<ElementId>,
    kind: BubbleKind,
    content: impl IntoElement,
    cx: &App,
) -> Stateful<Div> {
    let p = ramflux_desktop_navigation::palette(cx);
    let (fill, ink) = match kind {
        BubbleKind::Own => (p.outgoing, p.outgoing_text),
        BubbleKind::Peer | BubbleKind::Joined => (p.incoming, p.text),
    };
    div()
        .id(id)
        .flex()
        .flex_col()
        .min_w_0()
        .py(design_px(INSET_PX))
        .px(space(3.))
        .rounded(design_px(p.radius_bubble))
        .bg(fill)
        .text_color(ink)
        .child(content)
}

/// The author's avatar beside a bubble (boards 00B, 02): 32 px, a circle
/// for a person and a rounded square for an agent, the initial in 13 px
/// semibold.
pub fn message_avatar(initial: gpui_kit::SharedString, agent: bool, cx: &App) -> impl IntoElement {
    let theme = ramflux_desktop_navigation::palette(cx);
    let (fill, ink, radius) = if agent {
        (theme.agent_av, theme.on_agent_av, theme.radius_md)
    } else {
        (theme.avatar_person, theme.on_avatar_person, theme.radius_full)
    };
    div()
        .size(design_px(AVATAR_PX))
        .flex()
        .items_center()
        .justify_center()
        .rounded(design_px(radius))
        .bg(fill)
        .text_color(ink)
        .design_text(style::secondary().weight(gpui_kit::FontWeight::SEMIBOLD))
        .child(initial)
}

/// The avatar beside a bubble.
pub const AVATAR_PX: f32 = 32.0;
