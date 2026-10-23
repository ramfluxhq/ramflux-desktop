// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Message moments: text, delivery receipt (outbound), tombstones and an
//! attachment row. The text comes from the store's body cache; while it is
//! not in memory a placeholder shows and the conversation asks for it.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, SharedString, div};
use ramflux_desktop_core::{MessagePayload, Receipt};

use super::RowRefs;
use crate::bubble::EventSink;
use crate::i18n::{self, tr};

/// Renders a message body.
pub fn render(
    payload: &MessagePayload,
    body: Option<&SharedString>,
    refs: &RowRefs,
    outbound: bool,
    _sink: &EventSink,
    cx: &App,
) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    if payload.tombstone {
        return v_flex()
            .id("message-body")
            .text_sm()
            .italic()
            .text_color(muted)
            .child(tr("moment.message.deleted"));
    }
    let attachment = payload.attachment.as_ref().map(|id| {
        let object = refs.objects.get(id);
        let name = object.map_or_else(|| id.to_string(), |o| o.name.clone());
        let detail =
            object.map(|o| format!("{} · {}", i18n::bytes(o.size), i18n::transfer(&o.transfer)));
        h_flex()
            .id("message-attachment")
            .gap_2()
            .px_2()
            .py_1()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .child(Icon::new(IconName::File).small().text_color(muted))
            .child(div().text_sm().text_color(cx.theme().foreground).child(name))
            .when_some(detail, |row, detail| {
                row.child(div().text_xs().text_color(muted).child(detail))
            })
            .test_support()
    });
    let receipt_color = match payload.receipt {
        Receipt::Read => cx.theme().success,
        Receipt::Sending | Receipt::Sent | Receipt::Delivered => muted,
    };
    let text = body.cloned();
    v_flex()
        .id("message-body")
        .gap_1()
        .text_sm()
        .text_color(cx.theme().foreground)
        .when_some(text.filter(|t| !t.is_empty()), |col, text| col.child(text))
        .when(body.is_none(), |col| {
            col.child(
                div()
                    .id("message-not-loaded")
                    .italic()
                    .text_color(muted)
                    .child(tr("moment.message.not_loaded"))
                    .test_support(),
            )
        })
        .children(attachment)
        .when(outbound || payload.receipt == Receipt::Sending, |body| {
            body.child(
                h_flex()
                    .id("message-receipt")
                    .gap_1()
                    .text_xs()
                    .text_color(receipt_color)
                    .child(
                        Icon::new(match payload.receipt {
                            Receipt::Sending => IconName::Loader,
                            Receipt::Sent => IconName::Check,
                            Receipt::Delivered | Receipt::Read => IconName::CircleCheck,
                        })
                        .xsmall(),
                    )
                    .child(i18n::receipt(payload.receipt))
                    .test_support(),
            )
        })
}
