// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The inline confirm strip for high-risk intents (rule B4).
//!
//! The strip sits inline (above the composer, under the entry node card, at
//! the foot of settings); it is never a modal dialog. It owns only the
//! decision: the pending intent stays with the intent root, which re-submits
//! it with the human's confirmation when the strip reports
//! [`ConfirmEvent::Confirmed`]. The host draws the facts, then Cancel first
//! and Confirm beside it. One click on Confirm, or one press of Enter or
//! Space while it has the focus, confirms; Cancel or Escape dismisses. The
//! key that opened the strip never confirms it: GPUI counts a keyboard click
//! only when the key went down while the button already had the focus.

use crate::actions::{CONFIRM_CONTEXT, CancelConfirmation};
use crate::i18n::{self, tr, tr_args};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, ThemeStyled as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, Render, Role, Window, div,
};

mod request;

pub use request::ConfirmRequest;

/// Reported to the owner of the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmEvent {
    /// The human pressed Confirm; the owner submits the confirmed intent.
    Confirmed,
    /// The user cancelled; the owner discards the intent.
    Cancelled,
}

/// The confirm strip.
pub struct ConfirmStrip {
    request: Option<ConfirmRequest>,
    focus_handle: FocusHandle,
}

impl EventEmitter<ConfirmEvent> for ConfirmStrip {}

impl Focusable for ConfirmStrip {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ConfirmStrip {
    /// A closed strip.
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self { request: None, focus_handle: cx.focus_handle().tab_stop(true) }
    }

    /// Shows `request` and moves keyboard focus to its Confirm button.
    pub fn open(&mut self, request: ConfirmRequest, window: &mut Window, cx: &mut Context<Self>) {
        self.request = Some(request);
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// Hides the strip without emitting anything.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.request = None;
        cx.notify();
    }

    /// Whether a request is showing.
    pub fn is_open(&self) -> bool {
        self.request.is_some()
    }

    /// The showing request.
    pub fn request(&self) -> Option<&ConfirmRequest> {
        self.request.as_ref()
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        if self.request.take().is_some() {
            cx.emit(ConfirmEvent::Confirmed);
            cx.notify();
        }
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.request.take().is_some() {
            cx.emit(ConfirmEvent::Cancelled);
            cx.notify();
        }
    }

    fn on_cancel_action(&mut self, _: &CancelConfirmation, _: &mut Window, cx: &mut Context<Self>) {
        if self.request.is_some() {
            self.cancel(cx);
        } else {
            cx.propagate();
        }
    }
}

impl Render for ConfirmStrip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(request) = self.request.clone() else {
            return div().id("confirm-strip").into_any_element();
        };
        let accent = if request.risk().requires_confirmation() {
            cx.theme().danger
        } else {
            cx.theme().warning
        };
        let confirm_label = tr("confirm.accept");
        let risk = i18n::risk(request.risk());
        let announce =
            tr_args("confirm.announce", &[("summary", request.summary()), ("risk", &risk)]);
        let focused = self.focus_handle.is_focused(window);
        h_flex()
            .id("confirm-strip")
            .role(Role::Alert)
            .aria_label(announce)
            .test_support()
            .key_context(CONFIRM_CONTEXT)
            .on_action(cx.listener(Self::on_cancel_action))
            .flex_shrink_0()
            .mx_3()
            .mt_2()
            .gap_3()
            .px_3()
            .py_2()
            .border_1()
            .border_color(accent)
            .rounded(cx.theme().radius_lg)
            .bg(cx.theme().group_box)
            .child(Icon::new(IconName::TriangleAlert).text_color(accent))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().foreground)
                            .child(request.summary().clone()),
                    )
                    .when_some(request.detail().cloned(), |col, detail| {
                        col.child(
                            div()
                                .text_xs()
                                .font_family(cx.theme().mono_font_family.clone())
                                .text_color(cx.theme().muted_foreground)
                                .child(detail),
                        )
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .text_xs()
                            .child(div().text_color(accent).child(tr_args(
                                "confirm.risk",
                                &[("risk", &i18n::risk(request.risk()))],
                            )))
                            .when_some(request.proposed_by().cloned(), |row, agent| {
                                row.child(div().text_color(cx.theme().muted_foreground).child(
                                    tr_args("confirm.proposed_by_agent", &[("agent", &agent)]),
                                ))
                            }),
                    ),
            )
            .child(
                Button::new("confirm-cancel")
                    .outline()
                    .icon(IconName::Close)
                    .label(tr("common.cancel"))
                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
            )
            .child(
                div()
                    .id("confirm-accept")
                    .role(Role::Button)
                    .aria_label(confirm_label.clone())
                    .aria_description(tr("confirm.keys"))
                    .aria_keyshortcuts("Enter")
                    .test_support()
                    .track_focus(&self.focus_handle)
                    .h_8()
                    .px_3()
                    .flex()
                    .items_center()
                    .border_1()
                    .border_color(accent)
                    .rounded(cx.theme().radius)
                    .bg(accent.opacity(0.15))
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .cursor_pointer()
                    .when(focused, |control| control.focus_ring_style(window, cx))
                    .child(confirm_label)
                    // A click, or Enter or Space pressed and released while
                    // the button has the focus (GPUI's keyboard click).
                    .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))),
            )
            .into_any_element()
    }
}
