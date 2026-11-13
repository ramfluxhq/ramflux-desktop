// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Stateless two-line conversation content; a Kit Button owns row interaction.

use crate::theme;
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, h_flex, v_flex};
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
        }
    }

    /// Composes content only; the host supplies Button, stable ID and callback.
    pub fn content(self, cx: &App) -> impl IntoElement {
        let colors = theme::colors(cx);
        let initial = self.title.chars().next().map_or_else(|| "?".to_owned(), |c| c.to_string());
        let (background, foreground) = match self.kind {
            AvatarKind::Person => (colors.avatar, colors.on_avatar),
            AvatarKind::Agent => (colors.agent, colors.on_agent),
        };
        h_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .child(
                div()
                    .flex_none()
                    .size(rems(40.0 / 15.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(if self.kind == AvatarKind::Agent {
                        rems(10.0 / 15.0)
                    } else {
                        rems(20.0 / 15.0)
                    })
                    .bg(background)
                    .text_color(foreground)
                    .child(initial),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div().flex_1().min_w_0().truncate().font_medium().child(self.title),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
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
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(self.preview),
                            )
                            .when(self.attention > 0, |row| {
                                row.child(div().text_xs().child(format!("! {}", self.attention)))
                            })
                            .when(self.unread > 0, |row| {
                                row.child(
                                    div()
                                        .flex_none()
                                        .px_1()
                                        .rounded(cx.theme().radius)
                                        .bg(cx.theme().primary)
                                        .text_color(cx.theme().primary_foreground)
                                        .text_xs()
                                        .child(self.unread.to_string()),
                                )
                            }),
                    ),
            )
    }
}
