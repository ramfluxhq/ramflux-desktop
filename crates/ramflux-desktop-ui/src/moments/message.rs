// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Message moments: text, delivery receipt (outbound), tombstones and an
//! attachment row. The text comes from the store's body cache; while it is
//! not in memory a placeholder shows and the conversation asks for it.
//!
//! The attachment row shows the object's transfer state: a working
//! indicator while it moves (no percentage the source did not report), and
//! the reason when it failed.
//!
//! A file a message carries (a media row) is shown with its object when
//! the source reports one: an image as the image once its bytes are here,
//! any other file as a file row; the bytes moved while it travels; and the
//! actions its state allows (Download, Cancel, Retry, Save). A file that
//! was not sent says why; when the contact's home node is not pinned, the
//! row offers the contacts dock to exchange contact cards.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, SharedString, div};
use ramflux_desktop_core::{
    BlockId, Direction, MediaState, MediaView, MessagePayload, Object, ObjectAction, Scope,
    TransferState, UiEventKind,
};
use ramflux_desktop_navigation::direction::{DirectionalExt as _, h_flex};
use ramflux_desktop_navigation::scale::{
    DesignText as _, TextStyle, design_px, space, style, text_caption, text_secondary,
};

use super::{MomentRow, RowState, message_text};
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args};
use crate::style::DesignButton as _;

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
        return v_flex()
            .id("message-body")
            .text_size(text_secondary())
            .italic()
            .text_color(muted)
            .child(
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
            // A file name never carries directional controls (a spoofed
            // extension); it is shown as it reads.
            .map_or_else(
                || tr("moment.message.attachment"),
                |name| crate::i18n::bidi::strip_controls(name).into_owned().into(),
            );
        let moving =
            object.is_some_and(|o| matches!(o.transfer, TransferState::Transferring { .. }));
        let failed = object.is_some_and(|o| matches!(o.transfer, TransferState::Failed { .. }));
        let detail = object.map(|o| match &o.transfer {
            TransferState::Failed { reason } if reason != "paused" => i18n::join(
                &i18n::join(&i18n::size(o.size), &i18n::transfer(&o.transfer)),
                &i18n::backend_text(reason),
            )
            .to_string(),
            other => i18n::join(&i18n::size(o.size), &i18n::transfer(other)).to_string(),
        });
        let label = detail
            .as_ref()
            .map_or_else(|| name.to_string(), |detail| i18n::join(&name, detail).to_string());
        div()
            .id("message-attachment")
            .aria_label(label)
            .min_w_0()
            .w_full()
            .overflow_hidden()
            // Design B24 (message attachments): a file row inside the bubble, a 40 px
            // icon tile, the name over its size and state.
            .child(file_row(
                name,
                detail.unwrap_or_default().into(),
                outbound,
                h_flex()
                    .when(moving, |row| row.child(Icon::new(IconName::LoaderCircle).xsmall()))
                    .when(failed, |row| {
                        row.child(
                            Icon::new(IconName::TriangleAlert)
                                .xsmall()
                                .text_color(crate::tone::text(cx.theme().danger, cx)),
                        )
                    }),
                cx,
            ))
            .test_support()
    });
    let text = body.cloned();
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
        // The accessible name says it is a reply, as the visual quote does.
        let spoken = if refs.reply_excerpt.is_some() && !refs.reply_deleted {
            // The isolate marks only protect the visual layout of the excerpt.
            let text = label.trim_start_matches('\u{2068}').trim_end_matches('\u{2069}');
            i18n::tr_args("moment.message.reply", &[("text", text)])
        } else {
            label.clone()
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
            .accessibility_label(spoken)
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
            .text_size(text_caption())
            .italic()
            .text_color(muted)
            .child(label)
            .test_support()
    });
    let body_size = gpui_kit::rems(ramflux_desktop_navigation::palette(cx).size_body / 15.0);
    v_flex()
        .id("message-body")
        .gap(space(1.))
        // A message reads in its own direction, whatever the interface's.
        .text_start_in(super::body_direction(row))
        // Message text: 15 px on the boards' 24 px message line.
        .design_text(message_style())
        .text_color(foreground)
        .children(forwarded)
        .children(reply)
        .when_some(text.filter(|t| !t.is_empty()), |col, text| {
            if let Some(selection) = selection.filter(|_| payload.spans.is_empty()) {
                col.child(
                    // Kit insets a multi-line editor by its size's padding
                    // (4 px across, none down at the smallest size): the
                    // negative margin lines the text up with the sender.
                    div().id("message-text").test_support().min_w_0().mx(gpui_kit::px(-4.)).child(
                        gpui_kit::component::input::Textarea::new(selection)
                            .xsmall()
                            .text_size(body_size)
                            .line_height(design_px(message_style().line))
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
                    .text_size(text_caption())
                    .text_color(muted)
                    .child(tr("moment.message.edited"))
                    .test_support(),
            )
        })
        .when(!payload.reactions.is_empty(), |col| {
            col.child(
                h_flex()
                    .id("message-reactions")
                    .test_support()
                    .flex_wrap()
                    .gap(space(1.))
                    .children(payload.reactions.iter().map(|chip| {
                        let count = chip.count.to_string();
                        let key = if chip.mine {
                            "moment.message.reaction_mine"
                        } else {
                            "moment.message.reaction"
                        };
                        let label =
                            i18n::tr_args(key, &[("token", &chip.token), ("count", &count)]);
                        div()
                            .id(SharedString::from(format!("message-reaction-{}", chip.token)))
                            .aria_label(label.clone())
                            .px(space(2.))
                            .border_1()
                            .border_color(if chip.mine {
                                cx.theme().primary
                            } else {
                                cx.theme().border
                            })
                            .bg(if chip.mine { cx.theme().muted } else { cx.theme().background })
                            .rounded(cx.theme().radius)
                            .text_size(text_caption())
                            .text_color(cx.theme().foreground)
                            .child(label)
                            .test_support()
                    })),
            )
        })
        .children(payload.media.iter().enumerate().map(|(index, media)| {
            match refs.objects.get(&media.object_id) {
                Some(object) => media_file(index, object, outbound, sink, cx).into_any_element(),
                None => media_row(index, media, cx).into_any_element(),
            }
        }))
}

/// The reason keys after which a transfer cannot be tried again.
const FINAL_REASONS: [&str; 2] = ["backend.media.expired", "backend.media.deleted"];

/// The reason key of a send the contact's missing home node refused.
const NO_HOME_REASON: &str = "backend.media.no_home";

/// The width of an inline image and of a file row, in pixels.
pub(crate) const MEDIA_WIDTH_PX: f32 = 280.0;

/// A file of a message with its object: the image once its bytes are here
/// (for an image), the file row with its size and state, progress while it
/// moves, the reason when it failed, and the actions its state allows.
fn media_file(
    index: usize,
    object: &Object,
    outbound: bool,
    sink: &EventSink,
    cx: &App,
) -> impl IntoElement {
    let name: SharedString = object
        .name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        // A file name never carries directional controls (a spoofed
        // extension); it is shown as it reads.
        .map_or_else(
            || tr("moment.message.attachment"),
            |name| crate::i18n::bidi::strip_controls(name).into_owned().into(),
        );
    let moving = matches!(object.transfer, TransferState::Transferring { .. });
    let measured = matches!(object.transfer, TransferState::Transferring { transferred } if transferred > 0)
        && object.size.is_some_and(|s| s > 0);
    let failure = match &object.transfer {
        TransferState::Failed { reason } => Some(reason.clone()),
        _ => None,
    };
    let state = match &object.transfer {
        TransferState::Transferring { .. } if measured => i18n::join(
            &i18n::transfer(&object.transfer),
            &i18n::percent(f64::from(object.progress())),
        )
        .to_string(),
        TransferState::Complete if outbound => tr("moment.message.media.sent").to_string(),
        TransferState::Complete => tr("moment.message.media.downloaded").to_string(),
        TransferState::Queued => tr("moment.message.media.not_downloaded").to_string(),
        other => i18n::transfer(other).to_string(),
    };
    let detail = i18n::join(&i18n::size(object.size), &state).to_string();
    let label = i18n::join(&name, &detail).to_string();
    let image = (object.is_image() && matches!(object.transfer, TransferState::Complete))
        .then(|| object.local_path.clone())
        .flatten()
        .map(|path| {
            div()
                .id(SharedString::from(format!("message-media-image-{index}")))
                .aria_label(tr_args("moment.message.media.image", &[("name", &name)]))
                // A fixed 4:3 box: a finished download never changes the
                // row height, and a tiny image is still seen.
                .w(gpui_kit::px(MEDIA_WIDTH_PX))
                .h(gpui_kit::px(MEDIA_WIDTH_PX * 0.75))
                .rounded(gpui_kit::rems(ramflux_desktop_navigation::palette(cx).radius_md / 15.0))
                .overflow_hidden()
                .child(
                    gpui_kit::img(std::path::PathBuf::from(path))
                        .size_full()
                        .object_fit(gpui_kit::ObjectFit::Contain),
                )
                .test_support()
        });
    let action_sink = sink.scoped(Scope::Block(BlockId::new("object-actions")));
    let emit = |action: ObjectAction| {
        let sink = action_sink.clone();
        let object_id = object.id.clone();
        move |_: &gpui_kit::ClickEvent, window: &mut gpui_kit::Window, cx: &mut App| {
            sink.emit(
                UiEventKind::ObjectActionPressed {
                    object_id: object_id.clone(),
                    action: action.clone(),
                },
                window,
                cx,
            );
        }
    };
    let mut actions = h_flex().gap(space(2.)).flex_wrap();
    match &object.transfer {
        TransferState::Queued if object.direction == Direction::Inbound => {
            actions = actions.child(
                Button::new(SharedString::from(format!("message-media-download-{index}")))
                    .xsmall()
                    .design_primary(cx)
                    .icon(IconName::ArrowDown)
                    .design_label(tr("moment.message.media.download"))
                    .accessibility_label(tr_args(
                        "moment.message.media.download_named",
                        &[("name", &name)],
                    ))
                    .on_click(emit(ObjectAction::Pull)),
            );
        }
        TransferState::Transferring { .. } => {
            actions = actions.child(
                Button::new(SharedString::from(format!("message-media-cancel-{index}")))
                    .xsmall()
                    .design_secondary(cx)
                    .design_label(tr("moment.message.media.cancel"))
                    .on_click(emit(ObjectAction::Cancel)),
            );
        }
        TransferState::Failed { reason } if !FINAL_REASONS.contains(&reason.as_str()) => {
            if reason == NO_HOME_REASON {
                actions = actions.child(
                    Button::new(SharedString::from(format!("message-media-cards-{index}")))
                        .xsmall()
                        .design_primary(cx)
                        .design_label(tr("moment.message.media.exchange_cards"))
                        .on_click(|_, window, cx| {
                            window.dispatch_action(
                                crate::workspace::DockTool::Contacts.toggle_action(),
                                cx,
                            );
                        }),
                );
            }
            actions = actions.child(
                Button::new(SharedString::from(format!("message-media-retry-{index}")))
                    .xsmall()
                    .design_secondary(cx)
                    .icon(IconName::RotateCw)
                    .design_label(tr("moment.object.retry"))
                    .on_click(emit(ObjectAction::Retry)),
            );
        }
        TransferState::Complete if object.local_path.is_some() => {
            actions = actions.child(
                Button::new(SharedString::from(format!("message-media-save-{index}")))
                    .xsmall()
                    .design_secondary(cx)
                    .design_label(tr("moment.object.save"))
                    .on_click(super::object_transfer::save_with_dialog(&action_sink, object)),
            );
        }
        _ => {}
    }
    let danger = crate::tone::text(cx.theme().danger, cx);
    v_flex()
        .id(SharedString::from(format!("message-media-{index}")))
        .aria_label(label)
        .min_w(gpui_kit::px(MEDIA_WIDTH_PX))
        .w_full()
        .gap(space(1.))
        .children(image)
        .child(file_row(
            name.clone(),
            detail.into(),
            outbound,
            h_flex()
                .when(moving && !measured, |row| {
                    row.child(Icon::new(IconName::LoaderCircle).xsmall())
                })
                .when(failure.is_some(), |row| {
                    row.child(Icon::new(IconName::TriangleAlert).xsmall().text_color(danger))
                }),
            cx,
        ))
        .when(measured, |col| {
            col.child(
                gpui_kit::component::progress::Progress::new(SharedString::from(format!(
                    "message-media-progress-{index}"
                )))
                .value(object.progress() * 100.0)
                .accessibility_label(name.clone()),
            )
        })
        .when_some(failure, |col, reason| {
            let text = if outbound {
                i18n::tr_args(
                    "moment.message.media.not_sent",
                    &[("reason", &i18n::backend_text(&reason))],
                )
            } else {
                i18n::backend_text(&reason)
            };
            col.child(
                div()
                    .id(SharedString::from(format!("message-media-failure-{index}")))
                    .role(gpui_kit::Role::Status)
                    .aria_label(text.clone())
                    .text_size(text_caption())
                    .text_color(danger)
                    .child(text)
                    .test_support(),
            )
        })
        .when_some(object.saved_to.clone(), |col, path| {
            col.child(
                div()
                    .text_size(text_caption())
                    .child(tr_args("moment.object.saved_to", &[("path", &path)])),
            )
        })
        .child(actions)
        .test_support()
}

/// One media row of a v1 message: the file name and facts, the thumbnail
/// only when the policy kept one, and the reason when the object cannot be
/// opened.
fn media_row(index: usize, row: &MediaView, cx: &App) -> impl IntoElement {
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
        Some(state) => i18n::join(&i18n::size(Some(row.size)), state).to_string(),
        None => i18n::size(Some(row.size)),
    };
    let label = i18n::join(&name, &detail).to_string();
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
        .gap(space(1.))
        .children(thumbnail)
        .child(file_row(name, detail.into(), false, div(), cx))
        .test_support()
}

/// A file row (design `B24`): a 40 px icon tile, the name in the body size
/// over its facts in the meta size, and trailing state.
fn file_row(
    name: SharedString,
    detail: SharedString,
    outbound: bool,
    trailing: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    let palette = ramflux_desktop_navigation::palette(cx);
    let (background, text, meta) = if outbound {
        (palette.outgoing_quote, palette.outgoing_text, palette.outgoing_meta)
    } else {
        (palette.incoming_quote, palette.text, palette.incoming_meta)
    };
    h_flex()
        .w_full()
        .min_w_0()
        .gap(space(2.))
        .p(space(2.))
        .rounded(gpui_kit::rems(palette.radius_md / 15.0))
        .bg(background)
        .child(
            div()
                .flex_none()
                .size(gpui_kit::rems(40.0 / 15.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(gpui_kit::rems(palette.radius_sm / 15.0))
                .bg(palette.surface_white)
                .text_color(palette.text)
                .child(Icon::new(IconName::FileText).with_size(gpui_kit::px(20.))),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(gpui_kit::rems(palette.size_body / 15.0))
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(text)
                        .child(name),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(gpui_kit::rems(palette.size_meta / 15.0))
                        .text_color(meta)
                        .child(detail),
                ),
        )
        .child(trailing)
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

/// The message text style (boards 00B, 02, H04): the body size on a 24 px
/// line, with the 8 px bubble inset a one-line bubble is 40 px high.
pub(crate) fn message_style() -> TextStyle {
    style::body().line(MESSAGE_LINE_PX)
}

/// The message line of the boards.
const MESSAGE_LINE_PX: f32 = 24.0;

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
