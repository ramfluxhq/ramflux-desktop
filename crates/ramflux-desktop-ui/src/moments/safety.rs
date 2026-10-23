// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Safety change moments: a sticky banner while a key change or fork is
//! unresolved, with the safety number to compare and a Verify action; a
//! quiet record once verified.

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, div};
use ramflux_desktop_core::{
    BlockId, Lifecycle, SafetyChangeKind, SafetyChangePayload, Scope, UiEventKind,
};

use super::RowRefs;
use crate::bubble::EventSink;
use crate::i18n::{tr, tr_args};

/// Renders a safety change body.
pub fn render(
    payload: &SafetyChangePayload,
    refs: &RowRefs,
    lifecycle: Lifecycle,
    sink: &EventSink,
    cx: &App,
) -> impl IntoElement {
    let name = refs.name(&payload.participant_id);
    if payload.change == SafetyChangeKind::Verified {
        return h_flex()
            .id("safety-record")
            .gap_2()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(Icon::new(IconName::CircleCheck).small().text_color(cx.theme().success))
            .child(tr_args("moment.safety.verified", &[("name", &name)]))
            .into_any_element();
    }
    let open = lifecycle == Lifecycle::NeedsMe;
    let key = match payload.change {
        SafetyChangeKind::Forked => "moment.safety.forked",
        _ => "moment.safety.key_changed",
    };
    let number = refs.people.get(&payload.participant_id).and_then(|p| p.safety_number.clone());
    let verify_sink = sink.scoped(Scope::Block(BlockId::new("verify")));
    let participant_id = payload.participant_id.clone();
    let color = if open { cx.theme().danger } else { cx.theme().border };
    v_flex()
        .id("safety-banner")
        .gap_2()
        .p_3()
        .border_1()
        .border_color(color)
        .rounded(cx.theme().radius_lg)
        .bg(cx.theme().group_box)
        .child(
            h_flex()
                .gap_2()
                .child(Icon::new(IconName::TriangleAlert).small().text_color(color))
                .child(div().text_sm().font_medium().child(tr_args(key, &[("name", &name)]))),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(tr("moment.safety.explain")),
        )
        .when_some(number, |col, number| {
            col.child(
                div().text_sm().font_family(cx.theme().mono_font_family.clone()).child(number),
            )
        })
        .child(if open {
            Button::new("verify-safety")
                .small()
                .primary()
                .label(tr("moment.safety.verify"))
                .on_click(move |_, window, cx| {
                    verify_sink.emit(
                        UiEventKind::VerifySafetyPressed { participant_id: participant_id.clone() },
                        window,
                        cx,
                    );
                })
                .into_any_element()
        } else {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(tr("moment.safety.resolved"))
                .into_any_element()
        })
        .into_any_element()
}
