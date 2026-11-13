// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Stateless two-line conversation content; a Kit Button owns row interaction.

use crate::theme;
use gpui_kit::component::{StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, SharedString, div, rems};

/// Avatar shape is a source identity-kind fact, never a trust assertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvatarKind {
    /// Person or group: circular initials.
    Person,
    /// Agent: rounded-square initials.
    Agent,
}

/// Controlled presentation supplied by the account-scoped host.
#[non_exhaustive]
pub struct Row {
    /// Display title.
    pub title: SharedString,
    /// Current preview or localized unavailable state.
    pub preview: SharedString,
    /// Time if the source supplied it.
    pub time: SharedString,
    /// Unread messages, independent of pending decisions.
    pub unread: u32,
    /// Loaded pending decisions.
    pub attention: usize,
    /// Source kind; this does not mean verified.
    pub kind: AvatarKind,
    /// A fixed avatar glyph instead of the title's initial (the strangers
    /// entry shows "?").
    pub glyph: Option<SharedString>,
}

impl Row {
    /// Creates a row with no time, counts or inferred identity facts.
    #[must_use]
    pub fn new(title: SharedString, preview: SharedString) -> Self {
        Self {
            title,
            preview,
            time: SharedString::default(),
            unread: 0,
            attention: 0,
            kind: AvatarKind::Person,
            glyph: None,
        }
    }

    /// Composes content only; the host supplies Button, stable ID and callback.
    ///
    /// Geometry follows the design's `List row / Conversation`: a 40 px
    /// avatar, the name in the body size over a secondary-size preview, the
    /// time in the caption size and the unread count in a badge.
    pub fn content(self, cx: &App) -> impl IntoElement {
        let colors = theme::colors(cx);
        let palette = theme::palette(cx);
        let initial = self.glyph.clone().unwrap_or_else(|| {
            self.title
                .chars()
                .find(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
                .map_or_else(|| "?".into(), |c| c.to_string().into())
        });
        let (background, foreground) = match self.kind {
            AvatarKind::Person => (colors.avatar, colors.on_avatar),
            AvatarKind::Agent => (colors.agent, colors.on_agent),
        };
        let rem = |px: f32| rems(px / 15.0);
        h_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .child(
                div()
                    .flex_none()
                    .size(rem(crate::geometry::AVATAR))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(if self.kind == AvatarKind::Agent {
                        // The design's agent avatar: a 10 px corner on 40 px.
                        rem(crate::geometry::AVATAR / 4.0)
                    } else {
                        rem(palette.radius_full)
                    })
                    .bg(background)
                    .text_color(foreground)
                    .font_semibold()
                    .child(initial),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(rem(palette.size_body))
                                    .font_medium()
                                    .text_color(palette.text)
                                    .child(self.title),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(rem(palette.size_caption))
                                    .text_color(palette.secondary)
                                    .child(self.time),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(rem(palette.size_secondary))
                                    .text_color(palette.secondary)
                                    .child(self.preview),
                            )
                            .when(self.attention > 0, |row| {
                                row.child(
                                    div()
                                        .flex_none()
                                        .text_size(rem(palette.size_caption))
                                        .font_bold()
                                        .text_color(palette.text)
                                        .child(format!("! {}", self.attention)),
                                )
                            })
                            .when(self.unread > 0, |row| {
                                row.child(
                                    div()
                                        .flex_none()
                                        .min_w(rem(16.0))
                                        .px(rem(4.0))
                                        .flex()
                                        .justify_center()
                                        .rounded(rem(palette.radius_md))
                                        .bg(palette.badge)
                                        .text_color(palette.on_badge)
                                        .text_size(rem(palette.size_caption))
                                        .font_bold()
                                        .child(self.unread.to_string()),
                                )
                            }),
                    ),
            )
    }
}

/// Single-line untrusted label, bounded before copying and isolated from surrounding UI.
/// Scan at most 4096 scalars, even when a peer prefixes a huge body with controls.
#[must_use]
pub fn list_text(text: &str) -> SharedString {
    const SCAN_LIMIT: usize = 4096;
    const DISPLAY_LIMIT: usize = 200;
    let mut clean = String::new();
    let mut length = 0;
    let mut space = false;
    for c in text.chars().take(SCAN_LIMIT) {
        if c == '\n' || c == '\r' {
            if !clean.is_empty() {
                break;
            }
            continue;
        }
        if c.is_control()
            || matches!(c, '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{feff}')
            || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        {
            continue;
        }
        if c.is_whitespace() {
            space = !clean.is_empty();
            continue;
        }
        if length >= DISPLAY_LIMIT {
            clean.push('…');
            break;
        }
        if space {
            clean.push(' ');
            length += 1;
            space = false;
        }
        clean.push(c);
        length += 1;
    }
    if clean.is_empty() {
        return SharedString::default();
    }
    format!("\u{2068}{clean}\u{2069}").into()
}

/// First visible scalar of a sanitized identity, excluding directional isolates.
#[must_use]
pub fn initial(text: &str) -> SharedString {
    list_text(text)
        .chars()
        .find(|c| !matches!(c, '\u{2068}' | '\u{2069}') && !c.is_whitespace())
        .map_or_else(|| "?".into(), |c| c.to_string().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn labels_are_single_line_bounded_and_bidi_isolated() {
        assert_eq!(
            list_text("\n  \nمرحبا  world\u{202e}\t !\nsecret"),
            "\u{2068}مرحبا world !\u{2069}",
            "unsafe controls and later lines must not escape into the row"
        );
        const LARGE_BODY_BYTES: usize = 1_048_576;
        let label = list_text(&"x".repeat(LARGE_BODY_BYTES));
        assert!(label.chars().count() <= 203, "a megabyte body must produce a small snapshot");
        assert_eq!(list_text("\n\r\u{202e}"), "", "control-only input must remain empty");
    }
}
