// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Message moments: text, delivery state, and tombstones.

use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, div};
use ramflux_desktop_core::{MessagePayload, Receipt};

use crate::i18n::tr;

/// Renders a message body.
pub fn render(payload: &MessagePayload, cx: &App) -> impl IntoElement {
    if payload.tombstone {
        return v_flex()
            .text_sm()
            .italic()
            .text_color(cx.theme().muted_foreground)
            .child(tr("moment.message.deleted"));
    }
    v_flex().gap_1().text_sm().text_color(cx.theme().foreground).child(payload.text.clone()).when(
        payload.receipt == Receipt::Sending,
        |body| {
            body.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("moment.message.sending")),
            )
        },
    )
}
