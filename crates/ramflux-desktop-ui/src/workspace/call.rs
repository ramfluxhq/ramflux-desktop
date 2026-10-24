// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Voice calls in the workspace.
//!
//! - [`IncomingCall`]: the card that appears over the workspace while a
//!   call rings here, with Decline and Answer. It never takes focus from
//!   what the user is typing; Tab reaches its buttons.
//! - [`CallBar`]: the bar above the conversation field while this device is
//!   placing or in a call: who, where the call is (calling, connecting, its
//!   running duration), how it sounds, Mute and Hang up; and why there is no
//!   audio when there is none (microphone access denied, devices or media
//!   failed). Calls are headset-only until echo cancellation lands, which
//!   the bar says while a call runs.
//! - [`call_blocked`]: why the call button of a direct conversation is
//!   disabled.
//!
//! Both views read the store and the store handle's audio view; they emit
//! call UI events through the root sink and never change the store.

use std::time::Duration;

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, IntoElement, Render, Role, SharedString, Subscription, Task, Window, div,
};
use ramflux_desktop_core::{
    Call, CallState, ContextId, IntentError, Store, UiEventKind, check_start_call, methods,
    selectors,
};

use crate::app::{CallAudioView, CallQuality, StoreEvent, StoreHandle, wall_clock_now};
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args};
use crate::moments::call::duration_text;

/// System Settings, Privacy & Security, Microphone (macOS).
const MICROPHONE_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone";

/// Why a call cannot be placed in `context` now, translated; `None` when
/// it can. Calls exist only in direct conversations (no button elsewhere).
pub fn call_blocked(store: &Store, context: &ContextId) -> Option<SharedString> {
    if let Some(reason) = i18n::unsupported(store, methods::CALL_INVITE) {
        return Some(reason);
    }
    let error = check_start_call(store, context).err()?;
    Some(tr(match error {
        IntentError::CallInProgress => "call.blocked.in_progress",
        IntentError::SafetyUnresolved(_) => "call.blocked.safety",
        IntentError::NotReachable(_) => "call.blocked.unreachable",
        _ => "call.blocked.unavailable",
    }))
}

fn peer_name(store: &Store, call: &Call) -> SharedString {
    store.participant(&call.peer).map_or_else(
        || SharedString::from(call.peer.to_string()),
        |p| SharedString::from(p.display.clone()),
    )
}

/// The card of a call ringing here.
pub struct IncomingCall {
    store: Entity<StoreHandle>,
    sink: EventSink,
    _subscription: Subscription,
}

impl IncomingCall {
    /// A card reading `store`, emitting through `sink` (the root sink).
    pub fn new(store: Entity<StoreHandle>, sink: EventSink, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&store, |_, _, event: &StoreEvent, cx| {
            if let StoreEvent::Changed(changes) = event
                && changes.touches_calls()
            {
                cx.notify();
            }
        });
        Self { store, sink, _subscription: subscription }
    }

    /// The call the card shows: the earliest ringing one, while this device
    /// is in no other call.
    pub fn shown(store: &Store) -> Option<&Call> {
        if selectors::current_call(store).is_some() {
            return None;
        }
        selectors::ringing_calls(store).into_iter().next()
    }
}

impl Render for IncomingCall {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx).store();
        let Some(call) = Self::shown(store) else {
            return div().id("incoming-call").into_any_element();
        };
        let name = peer_name(store, call);
        let more = selectors::ringing_calls(store).len().saturating_sub(1);
        let answer_blocked = i18n::unsupported(store, methods::CALL_ANSWER);
        let (decline, answer) = (self.sink.clone(), self.sink.clone());
        let (decline_id, answer_id) = (call.id.clone(), call.id.clone());
        v_flex()
            .id("incoming-call")
            .role(Role::Alert)
            .aria_label(tr_args("call.incoming.a11y", &[("name", &name)]))
            .w_80()
            .gap_3()
            .p_4()
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius_lg)
            .shadow_lg()
            .child(
                h_flex()
                    .gap_3()
                    .child(Icon::new(IconName::PhoneIncoming).text_color(cx.theme().success))
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(tr("moment.call.incoming")),
                            )
                            .child(
                                div()
                                    .id("incoming-call-name")
                                    .font_semibold()
                                    .truncate()
                                    .child(name.clone())
                                    .test_support(),
                            ),
                    ),
            )
            .when(more > 0, |card| {
                card.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr_args("call.incoming.more", &[("count", &more.to_string())])),
                )
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("incoming-decline")
                            .small()
                            .danger()
                            .icon(IconName::PhoneOff)
                            .label(tr("call.decline"))
                            .on_click(move |_, window, cx| {
                                decline.emit(
                                    UiEventKind::CallAnswered {
                                        call_id: decline_id.clone(),
                                        accept: false,
                                    },
                                    window,
                                    cx,
                                );
                            }),
                    )
                    .child(
                        Button::new("incoming-answer")
                            .small()
                            .success()
                            .icon(IconName::Phone)
                            .label(tr("call.answer"))
                            .disabled(answer_blocked.is_some())
                            .when_some(answer_blocked, |button, reason| button.tooltip(reason))
                            .on_click(move |_, window, cx| {
                                answer.emit(
                                    UiEventKind::CallAnswered {
                                        call_id: answer_id.clone(),
                                        accept: true,
                                    },
                                    window,
                                    cx,
                                );
                            }),
                    ),
            )
            .test_support()
            .into_any_element()
    }
}

/// The bar of the call this device is placing or in.
pub struct CallBar {
    store: Entity<StoreHandle>,
    sink: EventSink,
    /// Re-renders every second while a call is under way (its duration).
    ticker: Option<Task<()>>,
    _subscription: Subscription,
}

impl CallBar {
    /// A bar reading `store`, emitting through `sink` (the root sink).
    pub fn new(store: Entity<StoreHandle>, sink: EventSink, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&store, |this, store, event: &StoreEvent, cx| {
            let relevant = match event {
                StoreEvent::Changed(changes) => changes.touches_calls(),
                StoreEvent::CallAudio => true,
                StoreEvent::BackgroundChanged(_) | StoreEvent::Inspected => false,
            };
            if relevant {
                let under_way = selectors::current_call(store.read(cx).store()).is_some();
                this.keep_ticking(under_way, cx);
                cx.notify();
            }
        });
        let mut bar = Self { store, sink, ticker: None, _subscription: subscription };
        let under_way = selectors::current_call(bar.store.read(cx).store()).is_some();
        bar.keep_ticking(under_way, cx);
        bar
    }

    fn keep_ticking(&mut self, on: bool, cx: &mut Context<Self>) {
        if !on {
            self.ticker = None;
        } else if self.ticker.is_none() {
            self.ticker = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }));
        }
    }
}

/// The bar's status text: calling, connecting, or the running duration.
pub fn call_status(call: &Call, now: ramflux_desktop_core::Timestamp) -> SharedString {
    match call.state {
        CallState::OutgoingRinging => tr("call.calling"),
        CallState::Connecting => tr("call.connecting"),
        CallState::Active => duration_text(now.saturating_since(call.answered_at.unwrap_or(now))),
        CallState::IncomingRinging => tr("moment.call.ringing"),
        CallState::Ended(_) => tr("call.ended"),
    }
}

/// Why there is no audio: a locale key (the desktop's or the backend's),
/// else the device's or engine's own words.
fn audio_reason(reason: &str) -> SharedString {
    if reason.starts_with("call.") { tr(reason) } else { i18n::backend_text(reason) }
}

fn quality(view: &CallAudioView) -> Option<(IconName, &'static str)> {
    let live = view.live?;
    Some(match live.quality {
        CallQuality::Connecting => (IconName::SignalZero, "call.quality.waiting"),
        CallQuality::Good => (IconName::SignalHigh, "call.quality.good"),
        CallQuality::Fair => (IconName::SignalMedium, "call.quality.fair"),
        CallQuality::Poor => (IconName::SignalLow, "call.quality.poor"),
        CallQuality::Lost => (IconName::SignalZero, "call.quality.lost"),
    })
}

impl Render for CallBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let handle = self.store.read(cx);
        let store = handle.store();
        let Some(call) = selectors::current_call(store).cloned() else {
            return div().id("call-bar").into_any_element();
        };
        let audio = handle.call_audio().filter(|a| a.call_id == call.id);
        let name = peer_name(store, &call);
        let status = call_status(&call, wall_clock_now());
        let muted = audio.as_ref().is_some_and(|a| a.muted);
        let ringing = call.state == CallState::OutgoingRinging;
        let blocked = audio.as_ref().and_then(|a| a.live).is_some_and(|l| l.microphone_blocked);
        let error = audio.as_ref().and_then(|a| a.error.clone());
        let signal = audio.as_ref().and_then(quality);
        let headset_hint = audio.as_ref().is_some_and(|a| a.live.is_some()) && !blocked;
        let hang_up = self.sink.clone();
        let call_id = call.id.clone();
        let store_entity = self.store.clone();
        let warn = |text: SharedString, action: Option<gpui_kit::AnyElement>, cx: &App| {
            h_flex()
                .id("call-bar-warning")
                .gap_2()
                .px_4()
                .py_1()
                .text_sm()
                .bg(cx.theme().warning.opacity(0.12))
                .child(Icon::new(IconName::TriangleAlert).small().text_color(cx.theme().warning))
                .child(div().flex_1().min_w_0().child(text))
                .children(action)
                .test_support()
        };
        let row = h_flex()
            .w_full()
            .gap_3()
            .px_4()
            .py_2()
            .child(Icon::new(IconName::PhoneCall).small().text_color(cx.theme().success))
            .child(
                div()
                    .id("call-bar-name")
                    .font_semibold()
                    .truncate()
                    .child(name.clone())
                    .test_support(),
            )
            .child(
                div()
                    .id("call-bar-status")
                    .aria_label(status.clone())
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(status)
                    .test_support(),
            )
            .when_some(signal, |row, (icon, key)| {
                row.child(
                    h_flex()
                        .id("call-bar-quality")
                        .aria_label(tr(key))
                        .gap_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(Icon::new(icon).xsmall())
                        .child(tr(key))
                        .test_support(),
                )
            })
            .child(div().flex_1())
            .when(headset_hint, |row| {
                row.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("call.headset")),
                )
            })
            .child(
                Button::new("call-mute")
                    .small()
                    .outline()
                    .selected(muted)
                    .icon(if muted { IconName::MicOff } else { IconName::Mic })
                    .label(tr(if muted { "call.unmute" } else { "call.mute" }))
                    .on_click(move |_, _, cx| {
                        store_entity.update(cx, |store, cx| store.set_call_muted(!muted, cx));
                    }),
            )
            .child(
                Button::new("call-hang-up")
                    .small()
                    .danger()
                    .icon(IconName::PhoneOff)
                    .label(tr(if ringing { "call.cancel" } else { "call.hang_up" }))
                    .on_click(move |_, window, cx| {
                        hang_up.emit(
                            UiEventKind::CallHangUpPressed { call_id: call_id.clone() },
                            window,
                            cx,
                        );
                    }),
            );
        let warning = if blocked {
            let open = cfg!(target_os = "macos").then(|| {
                Button::new("call-open-settings")
                    .small()
                    .outline()
                    .label(tr("call.microphone.open_settings"))
                    .on_click(|_, _, cx| cx.open_url(MICROPHONE_SETTINGS))
                    .into_any_element()
            });
            Some(warn(tr("call.microphone.blocked"), open, cx).into_any_element())
        } else {
            error.map(|reason| {
                let reason = audio_reason(&reason);
                warn(tr_args("call.audio.failed", &[("reason", &reason)]), None, cx)
                    .into_any_element()
            })
        };
        v_flex()
            .id("call-bar")
            .role(Role::Region)
            .aria_label(tr_args("call.bar.a11y", &[("name", &name)]))
            .w_full()
            .bg(cx.theme().success.opacity(0.08))
            .border_b_1()
            .border_color(cx.theme().border)
            .child(row)
            .children(warning)
            .test_support()
            .into_any_element()
    }
}
