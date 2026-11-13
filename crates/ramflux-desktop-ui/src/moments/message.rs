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
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, SharedString, div};
use ramflux_desktop_core::{
    MediaState, MediaView, MessagePayload, Receipt, TransferState, UiEventKind,
};

use super::{MomentRow, RowState, message_text};
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args};

/// Renders a message body.
pub fn render(
    payload: &MessagePayload,
    row: &MomentRow,
    state: &RowState,
    sink: &EventSink,
    cx: &App,
) -> impl IntoElement {
    let body = row.body.as_ref();
    let refs = &row.refs;
    let outbound = row.outbound;
    let selection = state.message_text.as_ref();
    let palette = ramflux_desktop_navigation::palette(cx);
    let foreground = if outbound { palette.outgoing_text } else { cx.theme().foreground };
    let muted = if outbound { palette.outgoing_meta } else { cx.theme().muted_foreground };
    if payload.tombstone {
        return v_flex().id("message-body").text_sm().italic().text_color(muted).child(
            div()
                .id("message-deleted")
                .aria_label(tr("moment.message.deleted"))
                .child(tr("moment.message.deleted"))
                .test_support(),
        );
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
        let label =
            detail.as_ref().map_or_else(|| name.to_string(), |detail| format!("{name} · {detail}"));
        div()
            .id("message-attachment")
            .aria_label(label)
            .min_w_0()
            .w_full()
            .overflow_hidden()
            .child(ramflux_desktop_navigation::generated::catalog::attachment_shell(
                name,
                detail.unwrap_or_default().into(),
                Icon::new(IconName::File).small(),
                h_flex()
                    .when(moving, |row| row.child(Icon::new(IconName::LoaderCircle).xsmall()))
                    .when(failed, |row| {
                        row.child(
                            Icon::new(IconName::TriangleAlert)
                                .xsmall()
                                .text_color(crate::tone::text(cx.theme().danger, cx)),
                        )
                    }),
                ramflux_desktop_navigation::palette(cx),
            ))
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
    let reply = payload.reply_to.as_ref().map(|target| {
        let label = if refs.reply_deleted {
            tr("message_ui.reply_deleted")
        } else {
            refs.reply_excerpt.as_deref().map_or_else(
                || {
                    tr(if refs.reply_available {
                        "message_ui.reply_not_loaded"
                    } else {
                        "moment.message.reply_missing"
                    })
                },
                ramflux_desktop_navigation::row::list_text,
            )
        };
        let jump = sink.clone();
        let target = target.message_id.clone();
        Button::new("message-reply")
            .ghost()
            .h_auto()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .p_0()
            .accessibility_label(label.clone())
            .disabled(!refs.reply_available)
            .on_click(move |_, window, cx| {
                jump.emit(
                    UiEventKind::FocusMomentRequested { moment_id: target.clone() },
                    window,
                    cx,
                )
            })
            .child(ramflux_desktop_navigation::generated::catalog::quote_excerpt(
                refs.reply_author.clone().unwrap_or_else(|| tr("message_ui.quoted")),
                label,
                ramflux_desktop_navigation::palette(cx),
            ))
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
        .text_color(foreground)
        .children(forwarded)
        .children(reply)
        .when_some(text.filter(|t| !t.is_empty()), |col, text| {
            if let Some(selection) = selection.filter(|_| payload.spans.is_empty()) {
                col.child(
                    div().id("message-text").test_support().min_w_0().child(
                        gpui_kit::component::input::Textarea::new(selection)
                            .text_sm()
                            .w_full()
                            .readonly(true)
                            .appearance(false)
                            .bordered(false)
                            .p_0()
                            .aria_label(tr("message_ui.body"))
                            .text_color(foreground),
                    ),
                )
            } else {
                col.child(message_text::render(&text, &payload.spans, foreground, cx))
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
        .when(payload.revision > 0, |col| {
            col.child(
                div()
                    .id("message-edited")
                    .aria_label(tr("moment.message.edited"))
                    .text_xs()
                    .text_color(muted)
                    .child(tr("moment.message.edited"))
                    .test_support(),
            )
        })
        .when(!payload.reactions.is_empty(), |col| {
            col.child(h_flex().id("message-reactions").test_support().flex_wrap().gap_1().children(
                payload.reactions.iter().map(|chip| {
                    let count = chip.count.to_string();
                    let key = if chip.mine {
                        "moment.message.reaction_mine"
                    } else {
                        "moment.message.reaction"
                    };
                    let label = i18n::tr_args(key, &[("token", &chip.token), ("count", &count)]);
                    div()
                        .id(SharedString::from(format!("message-reaction-{}", chip.token)))
                        .aria_label(label.clone())
                        .px_2()
                        .border_1()
                        .border_color(if chip.mine {
                            cx.theme().primary
                        } else {
                            cx.theme().border
                        })
                        .bg(if chip.mine { cx.theme().muted } else { cx.theme().background })
                        .rounded(cx.theme().radius)
                        .text_xs()
                        .text_color(cx.theme().foreground)
                        .child(label)
                        .test_support()
                }),
            ))
        })
        .children(payload.media.iter().enumerate().map(|(index, row)| media_row(index, row, cx)))
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

/// One media row of a v1 message: the file name and facts, the thumbnail
/// only when the policy kept one, and the reason when the object cannot be
/// opened.
fn media_row(index: usize, row: &MediaView, cx: &App) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    let name = if row.name.trim().is_empty() {
        tr("moment.message.attachment")
    } else {
        SharedString::from(row.name.clone())
    };
    let state = match row.state {
        MediaState::Available => None,
        MediaState::Revoked => Some(tr("moment.message.media.revoked")),
        MediaState::Unavailable => Some(tr("moment.message.media.unavailable")),
    };
    let detail = match &state {
        Some(state) => format!("{} \u{b7} {state}", i18n::size(Some(row.size))),
        None => i18n::size(Some(row.size)),
    };
    let label = format!("{name} \u{b7} {detail}");
    let thumbnail = thumbnail_image(row);
    let thumbnail = thumbnail.map(|(thumbnail, image)| {
        div()
            .id(SharedString::from(format!("message-media-thumbnail-{index}")))
            .aria_label(tr_args("moment.message.media.thumbnail", &[("name", &name)]))
            .child(
                gpui_kit::img(std::sync::Arc::new(image))
                    .max_w(gpui_kit::px(f32::from(u16::try_from(thumbnail.width).unwrap_or(512))))
                    .max_h(gpui_kit::px(f32::from(u16::try_from(thumbnail.height).unwrap_or(512)))),
            )
            .test_support()
    });
    v_flex()
        .id(SharedString::from(format!("message-media-{index}")))
        .aria_label(label)
        .gap_1()
        .children(thumbnail)
        .child(
            h_flex()
                .gap_2()
                .px_2()
                .py_1()
                .border_1()
                .border_color(cx.theme().border)
                .rounded(cx.theme().radius)
                .child(Icon::new(IconName::File).small().text_color(muted))
                .child(div().text_sm().text_color(cx.theme().foreground).child(name))
                .child(div().text_xs().text_color(muted).child(detail)),
        )
        .test_support()
}

/// The thumbnail of a row and its image. It is decoded as what its own bytes
/// are, not as the file's type: a JPEG attachment can carry a PNG preview.
fn thumbnail_image(row: &MediaView) -> Option<(&ramflux_desktop_core::Thumbnail, gpui_kit::Image)> {
    let thumbnail = row.thumbnail.as_ref()?;
    let format = image_format(&thumbnail.bytes)?;
    Some((thumbnail, gpui_kit::Image::from_bytes(format, thumbnail.bytes.clone())))
}

/// The codec of encoded image bytes, from their signature.
fn image_format(bytes: &[u8]) -> Option<gpui_kit::ImageFormat> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some(gpui_kit::ImageFormat::Png)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(gpui_kit::ImageFormat::Jpeg)
    } else if bytes.len() >= 12
        && bytes.get(..4) == Some(b"RIFF".as_slice())
        && bytes.get(8..12) == Some(b"WEBP".as_slice())
    {
        Some(gpui_kit::ImageFormat::Webp)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ramflux_desktop_test::v1_fixtures::{REAL_JPEG, REAL_PNG, REAL_WEBP};

    use super::*;

    /// Whether the image decoder accepts `bytes` as the format found in them.
    fn decodes(bytes: &[u8]) -> bool {
        let renderer = gpui_kit::SvgRenderer::new(Arc::new(crate::AppAssets));
        image_format(bytes)
            .map(|format| gpui_kit::Image::from_bytes(format, bytes.to_vec()))
            .is_some_and(|image| image.to_image_data(renderer).is_ok())
    }

    #[test]
    fn real_png_jpeg_and_webp_thumbnails_decode_as_their_own_codec() {
        assert!(decodes(REAL_PNG), "png");
        assert!(decodes(REAL_JPEG), "jpeg");
        assert!(decodes(REAL_WEBP), "webp");
    }

    #[test]
    fn a_jpeg_attachment_with_a_png_preview_decodes_the_preview() {
        let row = MediaView {
            object_id: ramflux_desktop_core::ObjectId::new("o-1"),
            name: "photo.jpg".to_owned(),
            mime: "image/jpeg".to_owned(),
            size: 5,
            state: MediaState::Available,
            thumbnail: Some(ramflux_desktop_core::Thumbnail {
                width: 1,
                height: 1,
                bytes: REAL_PNG.to_vec(),
            }),
        };
        let (_, image) = thumbnail_image(&row).expect("a thumbnail image");
        let renderer = gpui_kit::SvgRenderer::new(Arc::new(crate::AppAssets));
        assert!(image.to_image_data(renderer).is_ok(), "the PNG preview decodes");
    }

    #[test]
    fn the_codec_follows_the_thumbnail_bytes_not_the_file_type() {
        // A JPEG attachment carrying a PNG preview: the preview is a PNG.
        assert!(matches!(image_format(REAL_PNG), Some(gpui_kit::ImageFormat::Png)));
        assert!(matches!(image_format(REAL_JPEG), Some(gpui_kit::ImageFormat::Jpeg)));
        assert!(image_format(b"not an image").is_none());
    }
}
