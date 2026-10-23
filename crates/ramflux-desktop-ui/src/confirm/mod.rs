// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The inline hold-to-confirm strip for high-risk intents (rule B4).
//!
//! The strip sits inline (above the composer, under the entry node card, at
//! the foot of settings); it is never a modal dialog. It owns only the
//! gesture: the pending intent stays with the intent root, which re-submits
//! it with the human's confirmation when the strip reports
//! [`ConfirmEvent::HoldCompleted`]. A click is not a hold: the button must be
//! held (pointer, Space or Enter) for [`HOLD_DURATION`]; releasing early
//! resets it. The strip is an alert that names the request and its risk, and
//! the hold control reports its progress as a value from 0 to 100.

use std::time::{Duration, Instant};

use crate::actions::{CONFIRM_CONTEXT, CancelConfirmation};
use crate::i18n::{self, tr, tr_args};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, ThemeStyled as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent, KeyUpEvent,
    MouseButton, Render, Role, Task, Window, div, relative,
};

mod request;

pub use request::ConfirmRequest;

/// How long the confirm control must be held.
pub const HOLD_DURATION: Duration = Duration::from_millis(1_200);
const HOLD_FRAME: Duration = Duration::from_millis(30);

/// Reported to the owner of the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmEvent {
    /// The hold completed; the owner submits the confirmed intent.
    HoldCompleted {
        /// How long the control was held, in milliseconds.
        held_ms: u64,
    },
    /// The user cancelled; the owner discards the intent.
    Cancelled,
}

/// The hold-to-confirm strip.
pub struct ConfirmStrip {
    request: Option<ConfirmRequest>,
    hold_started: Option<Instant>,
    progress: f32,
    hold: Option<Task<()>>,
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
        Self {
            request: None,
            hold_started: None,
            progress: 0.0,
            hold: None,
            focus_handle: cx.focus_handle().tab_stop(true),
        }
    }

    /// Shows `request` and moves keyboard focus to the strip.
    pub fn open(&mut self, request: ConfirmRequest, window: &mut Window, cx: &mut Context<Self>) {
        self.request = Some(request);
        self.reset_hold();
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// Hides the strip without emitting anything.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.request = None;
        self.reset_hold();
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

    /// Hold progress from 0 to 1.
    pub fn progress(&self) -> f32 {
        self.progress
    }

    fn reset_hold(&mut self) {
        self.hold = None;
        self.hold_started = None;
        self.progress = 0.0;
    }

    fn begin_hold(&mut self, cx: &mut Context<Self>) {
        if self.request.is_none() || self.hold.is_some() {
            return;
        }
        self.hold_started = Some(cx.background_executor().now());
        self.hold = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(HOLD_FRAME).await;
                let done = this.update(cx, |strip, cx| strip.advance_hold(cx)).unwrap_or(true);
                if done {
                    break;
                }
            }
        }));
        cx.notify();
    }

    /// Returns true when the hold finished (completed or abandoned).
    fn advance_hold(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(started) = self.hold_started else {
            return true;
        };
        let held = cx.background_executor().now().saturating_duration_since(started);
        self.progress = (held.as_secs_f32() / HOLD_DURATION.as_secs_f32()).min(1.0);
        cx.notify();
        if held < HOLD_DURATION {
            return false;
        }
        self.request = None;
        self.hold_started = None;
        self.progress = 0.0;
        // Detach instead of dropping: this runs inside the hold task itself.
        if let Some(task) = self.hold.take() {
            task.detach();
        }
        let held_ms = u64::try_from(held.as_millis()).unwrap_or(u64::MAX);
        cx.emit(ConfirmEvent::HoldCompleted { held_ms });
        true
    }

    fn release_hold(&mut self, cx: &mut Context<Self>) {
        if self.hold_started.is_some() {
            self.reset_hold();
            cx.notify();
        }
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.request.take().is_some() {
            self.reset_hold();
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

/// Keys that press and hold the focused hold control.
fn is_hold_key(key: &str) -> bool {
    matches!(key, "space" | "enter")
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
        let progress = self.progress;
        let hold_label = tr("confirm.hold");
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
                div()
                    .id("hold-confirm")
                    .role(Role::Button)
                    .aria_label(hold_label.clone())
                    .aria_description(tr("confirm.hold_keys"))
                    .aria_keyshortcuts("Space")
                    .aria_numeric_value(f64::from(progress * 100.))
                    .aria_min_numeric_value(0.)
                    .aria_max_numeric_value(100.)
                    .test_support()
                    .track_focus(&self.focus_handle)
                    .relative()
                    .overflow_hidden()
                    .h_8()
                    .px_3()
                    .flex()
                    .items_center()
                    .border_1()
                    .border_color(accent)
                    .rounded(cx.theme().radius)
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .when(focused, |control| control.focus_ring_style(window, cx))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .h_full()
                            .w(relative(progress))
                            .bg(accent.opacity(0.3)),
                    )
                    .child(div().relative().child(hold_label))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.begin_hold(cx)),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.release_hold(cx)),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.release_hold(cx)),
                    )
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                        if is_hold_key(&event.keystroke.key) && !event.is_held {
                            this.begin_hold(cx);
                        }
                    }))
                    .on_key_up(cx.listener(|this, event: &KeyUpEvent, _, cx| {
                        if is_hold_key(&event.keystroke.key) {
                            this.release_hold(cx);
                        }
                    })),
            )
            .child(
                Button::new("confirm-cancel")
                    .ghost()
                    .small()
                    .icon(IconName::Close)
                    .label(tr("common.cancel"))
                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
            )
            .into_any_element()
    }
}
