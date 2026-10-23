// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! System and fallback notices: quiet status lines inside the conversation.

use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, div};
use ramflux_desktop_core::{FallbackPayload, SystemLevel, SystemPayload};

use crate::i18n::{self, tr};

/// Renders a system note. The level is named in text, not only by color.
pub fn system(payload: &SystemPayload, cx: &App) -> impl IntoElement {
    let (icon, color) = match payload.level {
        SystemLevel::Info => (IconName::Info, cx.theme().muted_foreground),
        SystemLevel::Warning => (IconName::TriangleAlert, cx.theme().warning),
        SystemLevel::Error => (IconName::CircleAlert, cx.theme().danger),
    };
    h_flex()
        .gap_2()
        .text_sm()
        .child(Icon::new(icon).small().text_color(color))
        .child(div().text_color(color).child(i18n::system_level(payload.level)))
        .child(div().min_w_0().text_color(cx.theme().foreground).child(payload.text.clone()))
}

/// Renders a capability downgrade notice.
pub fn fallback(payload: &FallbackPayload, cx: &App) -> impl IntoElement {
    v_flex()
        .gap_1()
        .text_sm()
        .child(
            h_flex()
                .gap_2()
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::EyeOff).small())
                .child(tr("moment.fallback.title"))
                .child(div().text_xs().child(crate::i18n::backend_text(&payload.reason))),
        )
        .child(div().text_color(cx.theme().foreground).child(payload.text.clone()))
}
