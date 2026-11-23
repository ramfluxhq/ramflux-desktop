// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Handoff moments: another device offers to continue the context here.
//! Accept or Decline while open; afterwards the target device is named.

use crate::style::DesignButton as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, div};
use ramflux_desktop_core::{BlockId, HandoffPayload, Lifecycle, Scope, UiEventKind};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};

use super::RowRefs;
use crate::bubble::EventSink;
use crate::i18n::{tr, tr_args};

/// Renders a handoff offer.
pub fn render(
    payload: &HandoffPayload,
    refs: &RowRefs,
    lifecycle: Lifecycle,
    sink: &EventSink,
    cx: &App,
) -> impl IntoElement {
    let from = refs.name(&payload.from_device);
    let answer = sink.scoped(Scope::Block(BlockId::new("answer")));
    let decline = answer.clone();
    let footer = match (lifecycle, &payload.to_device) {
        (Lifecycle::NeedsMe, _) => h_flex()
            .gap(space(2.))
            .child(
                Button::new("handoff-accept")
                    .small()
                    .design_primary(cx)
                    .design_label(tr("moment.handoff.accept"))
                    .on_click(move |_, window, cx| {
                        answer.emit(UiEventKind::HandoffAnswered { accept: true }, window, cx)
                    }),
            )
            .child(
                Button::new("handoff-decline")
                    .small()
                    .design_secondary(cx)
                    .design_label(tr("moment.handoff.decline"))
                    .on_click(move |_, window, cx| {
                        decline.emit(UiEventKind::HandoffAnswered { accept: false }, window, cx)
                    }),
            )
            .into_any_element(),
        (_, Some(target)) => div()
            .text_size(text_caption())
            .text_color(cx.theme().muted_foreground)
            .child(tr_args("moment.handoff.accepted", &[("device", &refs.name(target))]))
            .into_any_element(),
        _ => div()
            .text_size(text_caption())
            .text_color(cx.theme().muted_foreground)
            .child(tr("moment.handoff.declined"))
            .into_any_element(),
    };
    v_flex()
        .id("handoff")
        .gap(space(2.))
        .p(space(3.))
        .border_1()
        .border_color(if lifecycle == Lifecycle::NeedsMe {
            cx.theme().warning
        } else {
            cx.theme().border
        })
        .rounded(cx.theme().radius_lg)
        .child(
            h_flex()
                .gap(space(2.))
                .child(
                    Icon::new(IconName::HardDrive).small().text_color(cx.theme().muted_foreground),
                )
                .child(
                    div()
                        .text_size(text_secondary())
                        .font_medium()
                        .child(tr_args("moment.handoff.offer", &[("device", &from)])),
                ),
        )
        .child(
            div()
                .text_size(text_secondary())
                .text_color(cx.theme().muted_foreground)
                .child(payload.summary.clone()),
        )
        .child(footer)
}
