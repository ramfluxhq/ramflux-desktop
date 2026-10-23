// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Privacy reveal moments: the content stays masked until the human reveals
//! it with a hold; the reveal is recorded in an audit note line.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, div};
use ramflux_desktop_core::{BlockId, Lifecycle, PrivacyRevealPayload, Scope, UiEventKind};

use super::RowRefs;
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args};

/// Characters shown in place of masked content.
const MASK: &str = "•••• •••• •••• ••••";

/// Renders a privacy reveal.
pub fn render(
    payload: &PrivacyRevealPayload,
    refs: &RowRefs,
    lifecycle: Lifecycle,
    sink: &EventSink,
    cx: &App,
) -> impl IntoElement {
    let revealed = payload.revealed.as_ref().zip(payload.content.as_ref());
    let reveal_sink = sink.scoped(Scope::Block(BlockId::new("reveal")));
    let body = match revealed {
        Some((_, content)) => div()
            .id("privacy-content")
            .text_sm()
            .font_family(cx.theme().mono_font_family.clone())
            // The only place revealed text is read; it is shown, never copied out.
            .child(content.expose().to_owned())
            .test_support()
            .into_any_element(),
        None => div()
            .id("privacy-mask")
            .text_sm()
            .font_family(cx.theme().mono_font_family.clone())
            .text_color(cx.theme().muted_foreground)
            .child(MASK)
            .test_support()
            .into_any_element(),
    };
    let audit = payload.revealed.as_ref().map(|record| {
        tr_args(
            "moment.privacy.audit",
            &[("name", &refs.name(&record.by)), ("time", &i18n::time_of_day(record.at))],
        )
    });
    let can_reveal = revealed.is_none() && !lifecycle.is_terminal();
    v_flex()
        .id("privacy-reveal")
        .gap_2()
        .p_3()
        .border_1()
        .border_color(cx.theme().border)
        .rounded(cx.theme().radius_lg)
        .child(
            h_flex()
                .gap_2()
                .child(
                    Icon::new(if revealed.is_some() { IconName::Eye } else { IconName::EyeOff })
                        .small()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(div().text_sm().font_medium().child(payload.label.clone())),
        )
        .child(body)
        .when(can_reveal, |col| {
            col.child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("privacy-reveal-button")
                            .small()
                            .outline()
                            .label(tr("moment.privacy.reveal"))
                            .on_click(move |_, window, cx| {
                                reveal_sink.emit(UiEventKind::RevealPressed, window, cx);
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr("moment.privacy.audited")),
                    ),
            )
        })
        .when_some(audit, |col, audit| {
            col.child(
                div()
                    .id("privacy-audit")
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(audit)
                    .test_support(),
            )
        })
}
