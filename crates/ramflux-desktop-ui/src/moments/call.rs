// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Voice call records in the timeline: one line per call that says which
//! way it went and how (its duration, or missed, declined, no answer...).
//! While a call rings or runs the line says so; answering and hanging up
//! live in the incoming-call sheet and the in-call bar, not here (one
//! call, one place to act on it). An ended call offers Call back.

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Hsla, IntoElement, SharedString, div};
use ramflux_desktop_core::{Call, CallDirection, CallOutcome, CallState, UiEventKind};

use super::RowRefs;
use crate::bubble::EventSink;
use crate::i18n::tr;

/// `m:ss`, or `h:mm:ss` from an hour on.
pub fn duration_text(ms: u64) -> SharedString {
    let secs = ms / 1000;
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}").into() } else { format!("{m}:{s:02}").into() }
}

/// The record's title, detail, icon and whether it reads as a miss.
pub fn summary(call: &Call) -> (SharedString, Option<SharedString>, IconName, bool) {
    let outgoing = call.direction == CallDirection::Outgoing;
    let title = |missed: bool| {
        tr(if missed {
            "moment.call.missed"
        } else if outgoing {
            "moment.call.outgoing"
        } else {
            "moment.call.incoming"
        })
    };
    let way = if outgoing { IconName::PhoneOutgoing } else { IconName::PhoneIncoming };
    match (call.state, call.outcome()) {
        (_, Some(CallOutcome::Answered { duration_ms })) => {
            (title(false), Some(duration_text(duration_ms)), way, false)
        }
        (_, Some(CallOutcome::Missed)) => (title(true), None, IconName::PhoneMissed, true),
        (_, Some(outcome)) => {
            let detail = match outcome {
                CallOutcome::Declined => "moment.call.declined",
                CallOutcome::NoAnswer => "moment.call.no_answer",
                CallOutcome::Busy => "moment.call.busy",
                CallOutcome::Cancelled => "moment.call.cancelled",
                CallOutcome::AnsweredElsewhere => "moment.call.answered_elsewhere",
                _ => "moment.call.failed",
            };
            (title(false), Some(tr(detail)), IconName::PhoneOff, false)
        }
        (CallState::OutgoingRinging | CallState::IncomingRinging, None) => {
            (title(false), Some(tr("moment.call.ringing")), way, false)
        }
        (CallState::Connecting, None) => {
            (title(false), Some(tr("moment.call.connecting")), IconName::PhoneCall, false)
        }
        (_, None) => (title(false), Some(tr("moment.call.active")), IconName::PhoneCall, false),
    }
}

/// Renders a call record.
pub fn render(refs: &RowRefs, sink: &EventSink, cx: &App) -> impl IntoElement {
    let Some(call) = refs.call.as_ref() else {
        return div().id("call-record").into_any_element();
    };
    let (title, detail, icon, missed) = summary(call);
    let color: Hsla = if missed { cx.theme().danger } else { cx.theme().muted_foreground };
    let call_back = call.state.is_ended().then(|| {
        let sink = sink.clone();
        Button::new("call-back")
            .small()
            .ghost()
            .icon(IconName::Phone)
            .label(tr("moment.call.call_back"))
            .disabled(!refs.call_back)
            .on_click(move |_, window, cx| sink.emit(UiEventKind::CallStartRequested, window, cx))
    });
    h_flex()
        .id("call-record")
        .gap_2()
        .text_sm()
        .child(Icon::new(icon).small().text_color(color))
        .child(
            div()
                .text_color(if missed { cx.theme().danger } else { cx.theme().foreground })
                .child(title),
        )
        .when_some(detail, |row, detail| {
            row.child(div().text_color(cx.theme().muted_foreground).child(detail))
        })
        .when_some(call_back, |row, button| row.child(button))
        .test_support()
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_as_a_clock() {
        assert_eq!(duration_text(0), "0:00");
        assert_eq!(duration_text(63_400), "1:03");
        assert_eq!(duration_text(3_725_000), "1:02:05");
    }
}
