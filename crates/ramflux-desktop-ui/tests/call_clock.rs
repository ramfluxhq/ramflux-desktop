// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The call bar counts a call's duration on the session clock (scenario
//! time on the mock source), not the wall clock, so screenshots and tests
//! of an active call are deterministic.

mod common;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use ramflux_desktop_core::{
    Call, CallDirection, CallId, CallState, ContextId, DomainEvent, ParticipantId, Timestamp,
};
use ramflux_desktop_ui::moments::call::duration_text;

#[gpui_kit::test]
fn the_call_bar_counts_on_the_session_clock(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    let store = store_handle(cx, &shell);
    let epoch = cx.update(|cx| {
        store.update(cx, |h, _| h.mock_mut().map(|m| m.epoch())).expect("a mock source")
    });
    let answered = Timestamp(epoch.0 + 3_000);
    let call = Call {
        id: CallId::new("call-clock"),
        context_id: ContextId::new("c-lena"),
        peer: ParticipantId::new("p-lena"),
        caller: ParticipantId::new("p-me"),
        direction: CallDirection::Outgoing,
        state: CallState::Active,
        version: 1,
        started_at: answered,
        answered_at: Some(answered),
        ended_at: None,
        order: None,
    };
    cx.update(|cx| {
        store.update(cx, |h, _| {
            h.mock_mut().expect("a mock source").inject(0, DomainEvent::CallUpserted(call));
        })
    });
    advance(cx, 100);
    show_context(cx, handle, &shell, "c-lena");
    advance(cx, 65_000);
    let shown = in_window(cx, handle, |window, _| {
        window.try_find("call-bar-status").and_then(|e| e.label().map(str::to_owned))
    });
    // 65.1 s of scenario time since the answer, whatever the wall clock says.
    assert_eq!(shown.as_deref(), Some(&*duration_text(65_100)));
}
