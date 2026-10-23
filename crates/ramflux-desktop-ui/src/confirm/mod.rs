// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The inline hold-to-confirm strip for high-risk intents (rule B4).
//!
//! The strip sits above the composer; it is never a modal dialog. It owns
//! only the gesture: the pending intent stays with the workspace root, which
//! re-submits it with human confirmation when the strip reports
//! [`ConfirmEvent::HoldCompleted`]. A click is not a hold: the button must be
//! held (pointer or Space) for [`HOLD_DURATION`]; releasing early resets it.

use std::time::{Duration, Instant};

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent, KeyUpEvent,
    MouseButton, Render, Role, SharedString, Task, Window, div, relative,
};
use ramflux_desktop_core::{GovernanceOp, Intent, IntentKind, MomentKind, Risk, Store};

use crate::actions::{CONFIRM_CONTEXT, CancelConfirmation};
use crate::i18n::{self, tr, tr_args};

/// How long the confirm control must be held.
pub const HOLD_DURATION: Duration = Duration::from_millis(1_200);
const HOLD_FRAME: Duration = Duration::from_millis(30);

/// What the strip shows about the pending request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmRequest {
    summary: SharedString,
    risk: Risk,
    proposed_by_agent: bool,
}

impl ConfirmRequest {
    /// Describes `intent` for the strip.
    pub fn describe(store: &Store, intent: &Intent) -> Self {
        let (summary, risk) = describe_kind(store, &intent.kind);
        let proposed_by_agent = matches!(intent.actor, ramflux_desktop_core::Actor::Agent(_));
        Self { summary, risk, proposed_by_agent }
    }

    /// One-line description.
    pub fn summary(&self) -> &SharedString {
        &self.summary
    }

    /// Risk level shown on the strip.
    pub fn risk(&self) -> Risk {
        self.risk
    }
}

fn describe_kind(store: &Store, kind: &IntentKind) -> (SharedString, Risk) {
    let name = |id: &ramflux_desktop_core::ParticipantId| {
        store.participant(id).map_or_else(|| id.to_string(), |p| p.display.clone())
    };
    match kind {
        IntentKind::Approve { grant_id, .. } => {
            let risk = store.grant(grant_id).map_or(Risk::High, |g| g.risk);
            let summary = store
                .moments()
                .find_map(|m| match &m.kind {
                    MomentKind::Intervention(p) if &p.grant_id == grant_id => {
                        Some(p.summary.clone())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| grant_id.to_string());
            (tr_args("confirm.approve", &[("summary", &summary)]), risk)
        }
        IntentKind::SurfaceAction { moment_id, action_id, .. } => {
            let action = store.moment(moment_id).and_then(|m| match &m.kind {
                MomentKind::AgentSurface(s) => {
                    s.actions.iter().find(|a| &a.id == action_id).cloned()
                }
                _ => None,
            });
            let (label, risk) =
                action.map_or((action_id.clone(), Risk::High), |a| (a.label, a.risk));
            (tr_args("confirm.surface_action", &[("action", &label)]), risk)
        }
        IntentKind::Governance { context_id, op, .. } => {
            let group = store
                .context(context_id)
                .map_or_else(|| context_id.to_string(), |c| c.title.clone());
            let target = op.target().map(name).unwrap_or_default();
            let key = match op {
                GovernanceOp::Kick { .. } => "confirm.governance.kick",
                GovernanceOp::Ban { .. } => "confirm.governance.ban",
                GovernanceOp::SetRole { .. } => "confirm.governance.set_role",
                GovernanceOp::AddMember { .. } => "confirm.governance.add_member",
                GovernanceOp::RemoveMember { .. } => "confirm.governance.remove_member",
                GovernanceOp::CreateInvite { .. } => "confirm.governance.create_invite",
            };
            (tr_args(key, &[("member", &target), ("group", &group)]), op.risk())
        }
        _ => (tr("confirm.generic"), Risk::High),
    }
}

/// Reported to the owner of the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmEvent {
    /// The hold completed; the owner submits the confirmed intent.
    HoldCompleted,
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
        cx.emit(ConfirmEvent::HoldCompleted);
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

impl Render for ConfirmStrip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(request) = self.request.clone() else {
            return div().id("confirm-strip").into_any_element();
        };
        let accent = if request.risk.requires_confirmation() {
            cx.theme().danger
        } else {
            cx.theme().warning
        };
        let progress = self.progress;
        let hold_label = tr("confirm.hold");
        h_flex()
            .id("confirm-strip")
            .key_context(CONFIRM_CONTEXT)
            .on_action(cx.listener(Self::on_cancel_action))
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
                            .child(request.summary.clone()),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .text_xs()
                            .child(div().text_color(accent).child(tr_args(
                                "confirm.risk",
                                &[("risk", &i18n::risk(request.risk))],
                            )))
                            .when(request.proposed_by_agent, |row| {
                                row.child(
                                    div()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(tr("confirm.proposed_by_agent")),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .id("hold-confirm")
                    .role(Role::Button)
                    .aria_label(hold_label.clone())
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
                    .focus_visible(|style| style.border_2())
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
                        if event.keystroke.key == "space" && !event.is_held {
                            this.begin_hold(cx);
                        }
                    }))
                    .on_key_up(cx.listener(|this, event: &KeyUpEvent, _, cx| {
                        if event.keystroke.key == "space" {
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
