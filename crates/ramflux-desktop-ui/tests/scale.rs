// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The conversation field with 20 000 messages in one context: the message
//! scroller stays virtualized (only a bounded number of rows render), the
//! tail stays followed and anchored, history scrolls back to the oldest
//! message, and bodies evicted from the bounded cache come back on demand.

mod common;

use std::time::Instant;

use common::*;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{Entity, ScrollDelta, TestAppContext, WindowHandle, point, px};
use ramflux_desktop_core::{
    ContextId, DomainEvent, Lifecycle, MAX_BODIES, MessagePayload, Moment, MomentKind,
    ParticipantId, Receipt, Timestamp,
};
use ramflux_desktop_test::scale::{LARGE_CONTEXT, large_context, large_message_id};
use ramflux_desktop_ui::Shell;

type Handle = WindowHandle<gpui_kit::base::Root>;

const MESSAGES: usize = 20_000;

/// Upper bound on rendered rows: a viewport plus overdraw, never the list.
const RENDERED_ROWS_BOUND: usize = 60;

fn rendered_rows(cx: &mut TestAppContext, handle: Handle) -> Vec<String> {
    in_window(cx, handle, |window, _| {
        window
            .elements()
            .into_iter()
            .filter(|e| e.visible())
            .filter_map(|e| e.path().last().map(ToString::to_string))
            .filter(|id| id.starts_with("moment-m-large-"))
            .collect()
    })
}

fn row_count(cx: &mut TestAppContext, shell: &Entity<Shell>) -> usize {
    let workspace = workspace(cx, shell);
    cx.update(|cx| workspace.read(cx).conversation().read(cx).row_count())
}

/// A message arriving in the large context at `offset` ms.
fn arriving(offset: u64, id: &str, text: &str) -> (u64, DomainEvent) {
    let moment = Moment {
        id: id.into(),
        context_id: ContextId::new(LARGE_CONTEXT),
        origin: ParticipantId::new("p-ana"),
        at: Timestamp(0),
        kind: MomentKind::Message(MessagePayload {
            text: text.to_owned(),
            receipt: Receipt::Sent,
            tombstone: false,
            attachment: None,
            franking: None,
        }),
        lifecycle: Lifecycle::Done,
    };
    (offset, DomainEvent::MomentAdded(Box::new(moment)))
}

fn opened(cx: &mut TestAppContext, later: Vec<(u64, DomainEvent)>) -> (Handle, Entity<Shell>) {
    let (handle, shell) = open(cx, "s02_daily");
    let store = store_handle(cx, &shell);
    let mut scenario = large_context(MESSAGES);
    scenario.timeline.extend(later);
    cx.update(|cx| store.update(cx, |store, cx| store.load_scenario(&scenario, cx)));
    let started = Instant::now();
    advance(cx, 100);
    show_context(cx, handle, &shell, LARGE_CONTEXT);
    println!("BENCH ui_load_and_show_20k: {:.1} ms", started.elapsed().as_secs_f64() * 1e3);
    (handle, shell)
}

#[gpui_kit::test]
fn twenty_thousand_messages_render_a_bounded_window_at_the_tail(cx: &mut TestAppContext) {
    let (handle, shell) = opened(cx, Vec::new());
    assert_eq!(row_count(cx, &shell), MESSAGES);
    let rows = rendered_rows(cx, handle);
    assert!(
        !rows.is_empty() && rows.len() <= RENDERED_ROWS_BOUND,
        "rendered {} of {MESSAGES} rows",
        rows.len()
    );
    let newest = format!("moment-{}", large_message_id(MESSAGES - 1));
    assert!(rows.contains(&newest), "the conversation opens at the tail: {rows:?}");
    let following = cx.update(|cx| {
        let workspace = shell.read(cx).workspace().clone();
        let scroller = workspace.read(cx).conversation().read(cx).scroller().clone();
        scroller.read(cx).is_following_tail()
    });
    assert!(following, "the tail is followed");
    let cached = with_store(cx, &shell, |s| s.bodies().len());
    assert!(cached <= MAX_BODIES, "bodies stay bounded: {cached}");
}

#[gpui_kit::test]
fn a_new_message_appends_one_row_and_keeps_the_tail_anchored(cx: &mut TestAppContext) {
    let (handle, shell) = opened(cx, vec![arriving(1_000, "m-large-new", "One more")]);
    in_window(cx, handle, |_, _| {});
    let started = Instant::now();
    advance(cx, 1_000);
    println!("BENCH ui_append_one_to_20k: {:.1} ms", started.elapsed().as_secs_f64() * 1e3);
    assert_eq!(row_count(cx, &shell), MESSAGES + 1);
    let rows = rendered_rows(cx, handle);
    assert!(rows.len() <= RENDERED_ROWS_BOUND);
    assert!(
        exists(cx, handle, "moment-m-large-new"),
        "following the tail, the new message is in view"
    );
}

#[gpui_kit::test]
fn scrolling_back_reaches_the_oldest_message_and_reloads_its_body(cx: &mut TestAppContext) {
    let (handle, shell) = opened(cx, vec![arriving(5_000, "m-large-late", "Late")]);
    let oldest = large_message_id(0);
    assert!(
        with_store(cx, &shell, |s| s.body(&oldest).is_none()),
        "the oldest body was evicted from the bounded cache"
    );
    let workspace = workspace(cx, &shell);
    let scroller = cx.update(|cx| workspace.read(cx).conversation().read(cx).scroller().clone());
    cx.update(|cx| scroller.update(cx, |state, cx| state.scroll_to_item(0, cx)));
    cx.run_until_parked();
    let rows = rendered_rows(cx, handle);
    assert!(rows.contains(&format!("moment-{oldest}")), "the oldest row renders: {rows:?}");
    assert!(rows.len() <= RENDERED_ROWS_BOUND);
    let not_following = cx.update(|cx| !scroller.read(cx).is_following_tail());
    assert!(not_following, "reading history leaves the tail");
    // The visible rows asked for their bodies; the source delivers them.
    advance(cx, 100);
    in_window(cx, handle, |_, _| {});
    advance(cx, 100);
    let shown = cx.update(|cx| workspace.read(cx).conversation().read(cx).has_body(&oldest));
    assert!(shown, "the evicted body is back in the row");

    // A wheel scroll in history keeps the reader's place when a message
    // arrives at the tail.
    let target = rendered_rows(cx, handle).into_iter().nth(2).expect("a row in view");
    in_window(cx, handle, move |window, cx| {
        window.scroll(target, ScrollDelta::Pixels(point(px(0.), px(-40.))), cx);
    });
    let before = rendered_rows(cx, handle);
    advance(cx, 5_000);
    assert_eq!(row_count(cx, &shell), MESSAGES + 1, "the late message arrived");
    let after = rendered_rows(cx, handle);
    assert_eq!(after.first(), before.first(), "the anchor row stays in place");
    assert!(!after.contains(&"moment-m-large-late".to_owned()), "the tail did not jump");
}
