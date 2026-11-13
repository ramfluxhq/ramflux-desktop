// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Object transfer moments: name, direction and size, live progress, and the
//! actions the transfer state allows (pull, preview, save, retry).
//!
//! Progress is shown only as far as it is known: a transfer whose source
//! reports no bytes moved (the daemon sends an attachment in one request)
//! shows a working indicator and no percentage. A failure reason that is a
//! locale key is translated.
//!
//! Preview is view-local (the moment expands in place to show the text
//! preview the source supplied); pull, save and retry are intents.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, div};
use ramflux_desktop_core::{
    BlockId, Direction, ObjectAction, ObjectTransferPayload, Scope, TransferState, UiEventKind,
};

use super::{RowRefs, RowState};
use crate::bubble::EventSink;
use crate::file_dialog::{self, StartDir};
use crate::i18n::{self, tr, tr_args};

/// Renders an object transfer body.
pub fn render(
    payload: &ObjectTransferPayload,
    refs: &RowRefs,
    sink: &EventSink,
    state: &RowState,
    cx: &App,
) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    let Some(object) = refs.objects.get(&payload.object_id).cloned() else {
        return v_flex()
            .id("object-transfer")
            .text_sm()
            .text_color(muted)
            .child(tr_args("moment.object.missing", &[("id", payload.object_id.as_str())]));
    };
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
    let preview_sink = sink.scoped(Scope::Block(BlockId::new("object-preview")));
    let expanded = state.expanded;
    let mut actions = h_flex().gap_2();
    match &object.transfer {
        TransferState::Queued if object.direction == Direction::Inbound => {
            actions = actions.child(
                Button::new("object-pull")
                    .small()
                    .primary()
                    .icon(IconName::ArrowDown)
                    .label(tr("moment.object.pull"))
                    .on_click(emit(ObjectAction::Pull)),
            );
        }
        TransferState::Complete => {
            if object.preview.is_some() {
                actions = actions.child(
                    Button::new("object-preview")
                        .small()
                        .ghost()
                        .icon(if expanded { IconName::EyeOff } else { IconName::Eye })
                        .label(tr(if expanded {
                            "moment.object.hide_preview"
                        } else {
                            "moment.object.preview"
                        }))
                        .on_click(move |_, window, cx| {
                            preview_sink.emit(
                                UiEventKind::ExpandToggled { expanded: !expanded },
                                window,
                                cx,
                            );
                        }),
                );
            }
            if object.saved_to.is_none() && object.direction == Direction::Inbound {
                actions = actions.child(
                    Button::new("object-save")
                        .small()
                        .outline()
                        .label(tr("moment.object.save"))
                        .on_click(save_with_dialog(&action_sink, &object)),
                );
            }
        }
        TransferState::Failed { .. } => {
            actions = actions.child(
                Button::new("object-retry")
                    .small()
                    .outline()
                    .icon(IconName::RotateCw)
                    .label(tr("moment.object.retry"))
                    .on_click(emit(ObjectAction::Retry)),
            );
        }
        _ => {}
    }
    let percent = object.progress() * 100.0;
    let measured =
        matches!(object.transfer, TransferState::Transferring { transferred } if transferred > 0);
    let status = match &object.transfer {
        TransferState::Transferring { .. } if measured => i18n::join(
            &i18n::transfer(&object.transfer),
            &i18n::percent(f64::from(object.progress())),
        )
        .to_string(),
        TransferState::Failed { reason } if reason != "paused" => {
            i18n::join(&i18n::transfer(&object.transfer), &i18n::backend_text(reason)).to_string()
        }
        other => i18n::transfer(other).to_string(),
    };
    let status_color = match object.transfer {
        TransferState::Failed { .. } => crate::tone::text(cx.theme().danger, cx),
        _ => muted,
    };
    // Design `02 · Attachment` / `H07`: a compact file block on the incoming
    // surface, a 40 px icon tile, the name over its direction and size.
    let palette = ramflux_desktop_navigation::palette(cx);
    v_flex()
        .id("object-transfer")
        .w_full()
        .max_w(gpui_kit::rems(448.0 / 15.0))
        .gap_2()
        .p_2()
        .bg(palette.incoming)
        .rounded(gpui_kit::rems(palette.radius_bubble / 15.0))
        .child(
            h_flex()
                .gap_2()
                .child(
                    div()
                        .flex_none()
                        .size(gpui_kit::rems(40.0 / 15.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(gpui_kit::rems(palette.radius_sm / 15.0))
                        .bg(palette.surface_white)
                        .child(Icon::new(IconName::FileText).with_size(gpui_kit::px(20.))),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(gpui_kit::rems(palette.size_body / 15.0))
                                .font_medium()
                                .truncate()
                                .child(object.label()),
                        )
                        .child(
                            div()
                                .text_size(gpui_kit::rems(palette.size_meta / 15.0))
                                .text_color(muted)
                                .child(i18n::join(
                                    &i18n::direction(object.direction),
                                    &i18n::size(object.size),
                                )),
                        ),
                )
                .when(
                    matches!(object.transfer, TransferState::Transferring { .. }) && !measured,
                    |row| row.child(Icon::new(IconName::LoaderCircle).small().text_color(muted)),
                )
                .child(
                    div()
                        .id("object-transfer-status")
                        .text_size(gpui_kit::rems(palette.size_meta / 15.0))
                        .text_color(status_color)
                        .child(status)
                        .test_support(),
                ),
        )
        .when(measured, |col| {
            col.child(
                Progress::new("object-progress").value(percent).accessibility_label(object.label()),
            )
        })
        .when_some(object.saved_to.clone(), |col, path| {
            col.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(tr_args("moment.object.saved_to", &[("path", &path)])),
            )
        })
        .when(expanded, |col| {
            col.when_some(object.preview.clone(), |col, preview| {
                col.child(
                    div()
                        .id("object-preview-text")
                        .p_2()
                        .rounded(cx.theme().radius)
                        .bg(cx.theme().muted)
                        .text_sm()
                        .child(preview)
                        .test_support(),
                )
            })
        })
        .child(actions)
}

/// Click handler that asks where to save `object` with the native dialog
/// and emits the save with the chosen path. Shared with the objects dock.
pub fn save_with_dialog(
    sink: &EventSink,
    object: &ramflux_desktop_core::Object,
) -> impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut App) + 'static {
    let sink = sink.clone();
    let (object_id, name) = (object.id.clone(), object.label());
    move |_, window, cx| {
        let (sink, object_id) = (sink.clone(), object_id.clone());
        file_dialog::save_as(StartDir::Downloads, &name, window, cx, move |path, window, cx| {
            sink.emit(
                UiEventKind::ObjectActionPressed { object_id, action: ObjectAction::Save { path } },
                window,
                cx,
            );
        });
    }
}
