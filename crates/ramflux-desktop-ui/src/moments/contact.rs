// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Contact request moments: who asks, their note, and Accept / Reject while
//! the request is open.

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, div};
use ramflux_desktop_core::{BlockId, ContactRequestPayload, Lifecycle, Scope, UiEventKind};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};

use super::RowRefs;
use crate::bubble::EventSink;
use crate::i18n::{tr, tr_args};

/// Renders a contact request.
pub fn render(
    payload: &ContactRequestPayload,
    refs: &RowRefs,
    lifecycle: Lifecycle,
    sink: &EventSink,
    cx: &App,
) -> impl IntoElement {
    let name = refs.name(&payload.from);
    let answer_sink = sink.scoped(Scope::Block(BlockId::new("answer")));
    let reject_sink = answer_sink.clone();
    let footer = match lifecycle {
        Lifecycle::NeedsMe => h_flex()
            .gap(space(2.))
            .child(
                Button::new("contact-accept")
                    .small()
                    .primary()
                    .label(tr("moment.contact.accept"))
                    .on_click(move |_, window, cx| {
                        answer_sink.emit(UiEventKind::ContactAnswered { accept: true }, window, cx);
                    }),
            )
            .child(
                Button::new("contact-reject")
                    .small()
                    .outline()
                    .label(tr("moment.contact.reject"))
                    .on_click(move |_, window, cx| {
                        reject_sink.emit(
                            UiEventKind::ContactAnswered { accept: false },
                            window,
                            cx,
                        );
                    }),
            )
            .into_any_element(),
        Lifecycle::Done => status(tr("moment.contact.accepted"), cx),
        _ => status(tr("moment.contact.rejected"), cx),
    };
    v_flex()
        .id("contact-request")
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
                    Icon::new(IconName::CircleUser).small().text_color(cx.theme().muted_foreground),
                )
                .child(
                    div()
                        .text_size(text_secondary())
                        .font_medium()
                        .child(tr_args("moment.contact.request", &[("name", &name)])),
                ),
        )
        .when(!payload.note.is_empty(), |col| {
            col.child(
                div()
                    .text_size(text_secondary())
                    .text_color(cx.theme().muted_foreground)
                    .child(payload.note.clone()),
            )
        })
        .child(footer)
}

fn status(text: gpui_kit::SharedString, cx: &App) -> gpui_kit::AnyElement {
    div()
        .text_size(text_caption())
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}
