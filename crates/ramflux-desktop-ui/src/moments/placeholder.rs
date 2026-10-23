// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Compact placeholder for moment kinds without a renderer in this build.

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, SharedString, div};
use ramflux_desktop_core::MomentKind;

use crate::i18n::{self, tr};

/// Renders the kind name and a short summary where the payload has one.
pub fn render(kind: &MomentKind, cx: &App) -> impl IntoElement {
    let summary: Option<SharedString> = match kind {
        MomentKind::AgentSurface(surface) => Some(surface.fallback_text.clone().into()),
        MomentKind::Intervention(payload) => Some(payload.summary.clone().into()),
        MomentKind::Handoff(payload) => Some(payload.summary.clone().into()),
        MomentKind::PrivacyReveal(payload) => Some(payload.label.clone().into()),
        _ => None,
    };
    h_flex()
        .gap_2()
        .px_2()
        .py_1()
        .text_sm()
        .border_1()
        .border_dashed()
        .border_color(cx.theme().border)
        .rounded(cx.theme().radius)
        .child(div().font_medium().text_color(cx.theme().foreground).child(i18n::moment_kind(kind)))
        .when_some(summary, |row, summary| {
            row.child(
                div().min_w_0().truncate().text_color(cx.theme().muted_foreground).child(summary),
            )
        })
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(tr("moment.placeholder.later")),
        )
}
