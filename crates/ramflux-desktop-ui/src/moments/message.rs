// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Message moments: text, delivery receipt (outbound), tombstones and an
//! attachment row. The text comes from the store's body cache; while it is
//! not in memory a placeholder shows and the conversation asks for it.
//!
//! The attachment row shows the object's transfer state: a working
//! indicator while it moves (no percentage the source did not report), and
//! the reason when it failed.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, SharedString, div};
use ramflux_desktop_core::{MessagePayload, Receipt, TransferState, UiEventKind};

use super::{RowRefs, message_text};
use crate::bubble::EventSink;
use crate::i18n::{self, tr};

/// Renders a message body.
pub fn render(
    payload: &MessagePayload,
    body: Option<&SharedString>,
    refs: &RowRefs,
    outbound: bool,
    sink: &EventSink,
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
        let name = object
            .and_then(|o| o.name.as_deref())
            .filter(|name| !name.trim().is_empty())
            .map_or_else(|| tr("moment.message.attachment"), |name| name.to_owned().into());
        let moving =
            object.is_some_and(|o| matches!(o.transfer, TransferState::Transferring { .. }));
        let failed = object.is_some_and(|o| matches!(o.transfer, TransferState::Failed { .. }));
        let detail = object.map(|o| match &o.transfer {
            TransferState::Failed { reason } if reason != "paused" => format!(
                "{} · {} · {}",
                i18n::size(o.size),
                i18n::transfer(&o.transfer),
                i18n::backend_text(reason)
            ),
            other => format!("{} · {}", i18n::size(o.size), i18n::transfer(other)),
        });
        let detail_color = if failed { crate::tone::text(cx.theme().danger, cx) } else { muted };
        let label =
            detail.as_ref().map_or_else(|| name.to_string(), |detail| format!("{name} · {detail}"));
        h_flex()
            .id("message-attachment")
            .aria_label(label)
            .gap_2()
            .px_2()
            .py_1()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .child(Icon::new(IconName::File).small().text_color(muted))
            .child(div().text_sm().text_color(cx.theme().foreground).child(name))
            .when(moving, |row| {
                row.child(Icon::new(IconName::LoaderCircle).xsmall().text_color(muted))
            })
            .when_some(detail, |row, detail| {
                row.child(div().text_xs().text_color(detail_color).child(detail))
            })
            .test_support()
    });
    let receipt_color = match payload.receipt {
        Receipt::Read => crate::tone::text(cx.theme().success, cx),
        Receipt::Sending | Receipt::WaitingForNode { .. } | Receipt::Sent | Receipt::Delivered => {
            muted
        }
        Receipt::Failed { .. } => crate::tone::text(cx.theme().danger, cx),
    };
    let text = body.cloned();
    let retry = sink.clone();
    let reply = payload.reply_to.as_ref().map(|_| {
        let label = refs.reply_excerpt.as_deref().map_or_else(
            || tr("moment.message.reply_missing"),
            |quoted| i18n::tr_args("moment.message.reply", &[("text", quoted)]),
        );
        div()
            .id("message-reply")
            .aria_label(label.clone())
            .px_2()
            .border_l_2()
            .border_color(cx.theme().border)
            .text_xs()
            .text_color(muted)
            .child(label)
            .test_support()
    });
    let forwarded = payload.forward.as_ref().map(|forward| {
        let label = forward.from_name.as_deref().map_or_else(
            || tr("moment.message.forwarded"),
            |name| i18n::tr_args("moment.message.forwarded_from", &[("name", name)]),
        );
        div()
            .id("message-forward")
            .aria_label(label.clone())
            .text_xs()
            .italic()
            .text_color(muted)
            .child(label)
            .test_support()
    });
    v_flex()
        .id("message-body")
        .gap_1()
        .text_sm()
        .text_color(cx.theme().foreground)
        .children(forwarded)
        .children(reply)
        .when_some(text.filter(|t| !t.is_empty()), |col, text| {
            if payload.spans.is_empty() {
                col.child(text)
            } else {
                col.child(message_text::render(&text, &payload.spans, cx))
            }
        })
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
        .when(
            outbound
                || matches!(
                    payload.receipt,
                    Receipt::Sending | Receipt::WaitingForNode { .. } | Receipt::Failed { .. }
                ),
            |body| {
                body.child(
                    h_flex()
                        .id("message-receipt")
                        .gap_1()
                        .text_xs()
                        .text_color(receipt_color)
                        .child(
                            Icon::new(match payload.receipt {
                                Receipt::WaitingForNode { .. } => IconName::Loader,
                                Receipt::Failed { .. } => IconName::TriangleAlert,
                                Receipt::Sending => IconName::Loader,
                                Receipt::Sent => IconName::Check,
                                Receipt::Delivered | Receipt::Read => IconName::CircleCheck,
                            })
                            .xsmall(),
                        )
                        .child(i18n::receipt(payload.receipt))
                        .when(payload.receipt == (Receipt::Failed { retryable: true }), |row| {
                            row.child(
                                Button::new("message-retry")
                                    .small()
                                    .label(tr("moment.message.retry"))
                                    .on_click(move |_, window, cx| {
                                        retry.emit(UiEventKind::RetryMessagePressed, window, cx)
                                    }),
                            )
                        })
                        .when_some(refs.send_retry_ms, |row, ms| {
                            row.child(crate::i18n::tr_args(
                                "moment.message.retry_in",
                                &[("seconds", &ms.div_ceil(1000).to_string())],
                            ))
                        })
                        .test_support(),
                )
            },
        )
}
