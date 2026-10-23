// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI integration tests of the core flows: the production window root in a
//! headless window, driven by real pointer and keyboard input and the
//! scenario clock. Each test asserts the intents and effects that resulted.

mod common;

use common::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use ramflux_desktop_core::{
    ContextId, GrantId, GrantState, IntentKind, Lifecycle, MomentId, ParticipantId, methods,
};
use ramflux_desktop_ui::app::{InspectorEntry, UiOutcome};
use ramflux_desktop_ui::route::Page;

fn grant_state(
    cx: &mut TestAppContext,
    shell: &gpui_kit::Entity<ramflux_desktop_ui::Shell>,
) -> GrantState {
    with_store(cx, shell, |store| store.grant(&GrantId::new("g-rotate")).expect("grant").state)
}

#[gpui_kit::test]
fn s02_send_message_and_receipts_advance(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    assert_eq!(page(cx, &shell), Page::Workspace);
    let workspace = workspace(cx, &shell);
    let current =
        |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).current_context().cloned());
    assert_eq!(current(cx), Some(ContextId::new("c-team")), "the pinned context opens first");

    show_context(cx, handle, &shell, "c-lena");
    type_into(cx, handle, "composer-input", "12:30 works");
    click(cx, handle, "composer-send");
    assert_eq!(effects(cx, &shell), [methods::SEND_MESSAGE]);
    let body = last_body(cx, &shell, methods::SEND_MESSAGE);
    assert_eq!(body["context_id"], "c-lena");
    assert_eq!(body["text"], "12:30 works");
    assert_eq!(body["proposed_by"], "Human");
    assert!(body["confirmed_by"].is_null());
    advance(cx, 100);
    assert_eq!(rows(cx, &shell).last().map(String::as_str), Some("m-lena-reply"));
    assert!(in_window(cx, handle, |window, _| window
        .find("composer-input")
        .value()
        .unwrap_or_default()
        .is_empty()));
    assert!(
        exists_in(cx, handle, "moment-m-lena-reply", "message-receipt"),
        "outbound messages show their receipt"
    );
    advance(cx, 2_000);
    let receipt = with_store(cx, &shell, |s| {
        format!("{:?}", s.moment(&MomentId::new("m-lena-reply")).map(|m| m.kind.clone()))
    });
    assert!(receipt.contains("Read"), "the receipt advanced: {receipt}");

    // Back to the pinned context through its stack entry.
    click(cx, handle, "stack-c-team");
    assert_eq!(current(cx), Some(ContextId::new("c-team")));
}

#[gpui_kit::test]
fn s02_day_headers_group_history_by_local_day(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    show_context(cx, handle, &shell, "c-lena");
    let workspace = workspace(cx, &shell);
    let headers = cx.update(|cx| workspace.read(cx).conversation().read(cx).day_headers());
    let today = ramflux_desktop_ui::i18n::tr("clock.today").to_string();
    assert!(headers.first().cloned().flatten().is_some(), "the first row opens a day");
    assert_eq!(
        headers.iter().flatten().filter(|h| **h == today).count(),
        1,
        "one today header: {headers:?}"
    );
    let times: Vec<String> = with_store(cx, &shell, |s| {
        ramflux_desktop_core::selectors::timeline(s, &ContextId::new("c-lena"))
            .iter()
            .map(|m| ramflux_desktop_ui::i18n::time_of_day(m.at))
            .collect()
    });
    assert!(times.iter().any(|t| t != &times[0]), "times are real wall-clock times: {times:?}");
}

#[gpui_kit::test]
fn s03_approval_opens_the_strip_and_a_hold_confirms(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s03_intervention");
    advance(cx, 1_000);

    click_in(cx, handle, "moment-m-approve", "approve");
    assert!(strip_open(cx, &shell), "high risk opens the inline strip");
    assert!(effects(cx, &shell).is_empty(), "nothing reaches the source before the hold");

    // A click is not a hold.
    click(cx, handle, "hold-confirm");
    advance(cx, 2_000);
    assert_eq!(grant_state(cx, &shell), GrantState::Pending);
    assert!(strip_open(cx, &shell));

    hold_confirm(cx, handle);
    assert_eq!(effects(cx, &shell), [methods::GRANT_APPROVE]);
    let body = last_body(cx, &shell, methods::GRANT_APPROVE);
    assert_eq!(body["proposed_by"], "Human");
    assert_eq!(body["confirmed_by"]["gesture"], "Hold");
    assert!(body["confirmed_by"]["held_ms"].as_u64().unwrap_or(0) >= 1_200);
    assert!(!strip_open(cx, &shell));
    advance(cx, 1_000);

    assert_eq!(grant_state(cx, &shell), GrantState::Approved);
    let lifecycle =
        with_store(cx, &shell, |s| s.moment(&MomentId::new("m-approve")).map(|m| m.lifecycle));
    assert_eq!(lifecycle, Some(Lifecycle::Done));
    assert!(
        rows(cx, &shell).contains(&"m-result".to_owned()),
        "the result returns to the conversation"
    );
    assert!(exists(cx, handle, "intervention-audit"), "the moment shows who proposed and the hold");
}

#[gpui_kit::test]
fn s03_agent_proposal_keeps_the_proposer_after_the_human_hold(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s03_intervention");
    advance(cx, 1_000);
    let workspace = workspace(cx, &shell);

    cx.update_window(handle.into(), |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            let kind = IntentKind::Approve { grant_id: GrantId::new("g-rotate") };
            workspace.submit_agent_intent(ParticipantId::new("p-orbit"), kind, window, cx);
        });
    })
    .expect("window");
    cx.run_until_parked();

    assert!(strip_open(cx, &shell), "an agent never confirms; the strip waits for the human");
    let proposer = cx.update(|cx| {
        let root = shell.read(cx).root().read(cx);
        root.confirm_strip().read(cx).request().and_then(|r| r.proposed_by().cloned())
    });
    assert_eq!(proposer.as_deref(), Some("Orbit"), "the strip names the proposing agent");
    advance(cx, 2_000);
    assert!(effects(cx, &shell).is_empty());
    assert_eq!(grant_state(cx, &shell), GrantState::Pending);

    hold_confirm(cx, handle);
    advance(cx, 500);
    let body = last_body(cx, &shell, methods::GRANT_APPROVE);
    assert_eq!(body["proposed_by"]["Agent"], "p-orbit", "the proposer is kept");
    assert_eq!(body["confirmed_by"]["gesture"], "Hold", "the human confirmed");
    assert_eq!(grant_state(cx, &shell), GrantState::Approved);
}

#[gpui_kit::test]
fn bubbling_produces_one_root_intent_and_expand_stays_local(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s03_intervention");
    advance(cx, 1_000);
    let store = store_handle(cx, &shell);
    let root_intents =
        |cx: &mut TestAppContext| cx.update(|cx| store.read(cx).inspector().root_intents().count());
    let workspace = workspace(cx, &shell);

    let before = root_intents(cx);
    click_in(cx, handle, "moment-m-approve", "details");
    assert_eq!(root_intents(cx), before, "an expand click produces no intent");
    let expanded = cx.update(|cx| {
        workspace.read(cx).conversation().read(cx).is_expanded(&MomentId::new("m-approve"))
    });
    assert!(expanded, "the moment level consumed the expand click");
    let consumed = cx.update(|cx| {
        store.read(cx).inspector().entries().any(|entry| {
            matches!(entry, InspectorEntry::UiEvent { outcome: UiOutcome::Consumed { at }, .. } if at == "Moment(m-approve)")
        })
    });
    assert!(consumed);

    click_in(cx, handle, "moment-m-approve", "approve");
    assert_eq!(root_intents(cx), before + 1, "exactly one root intent");
    let last = cx.update(|cx| store.read(cx).inspector().root_intents().last().cloned());
    let Some(InspectorEntry::UiEvent { path, outcome, .. }) = last else {
        panic!("expected a root UI event");
    };
    assert_eq!(path, "Workspace > Context(c-ops) > Moment(m-approve) > Block(decision)");
    assert!(matches!(outcome, UiOutcome::Intent { ref intent, .. } if intent == "Approve"));
}

#[gpui_kit::test]
fn dev_panel_seek_to_zero_resets_and_replays_to_the_same_state(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s02_daily");
    in_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(ramflux_desktop_ui::actions::ToggleDevPanel), cx);
    });
    assert!(cx.update(|cx| shell.read(cx).is_dev_panel_visible()));
    advance(cx, 3_000);
    let snapshot = |cx: &mut TestAppContext| with_store(cx, &shell, |s| format!("{s:?}"));
    let before = snapshot(cx);
    let clock = cx
        .update(|cx| shell.read(cx).store().read(cx).session().map(|s| s.clock_ms()))
        .expect("session");

    click(cx, handle, "dev-restart");
    let reset_seq = with_store(cx, &shell, |s| s.last_seq());
    assert!(reset_seq < 20, "seek to t=0 keeps only the events at offset 0");
    assert_ne!(snapshot(cx), before);

    let dev = cx.update(|cx| shell.read(cx).dev_panel().cloned()).expect("debug build");
    cx.update(|cx| dev.update(cx, |dev, cx| dev.seek(clock, cx)));
    cx.run_until_parked();
    assert_eq!(snapshot(cx), before, "replay reaches the same state");
    assert!(!rows(cx, &shell).is_empty(), "views refreshed after the reset");
}

#[gpui_kit::test]
fn go_to_needs_me_cycles_through_attention_items(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s07_conflict_privacy_handoff");
    advance(cx, 1_000);
    let workspace = workspace(cx, &shell);
    let highlighted = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            workspace.read(cx).conversation().read(cx).highlighted().map(ToString::to_string)
        })
    };
    click(cx, handle, "needs-me");
    assert_eq!(highlighted(cx).as_deref(), Some("m-conflict"));
    click(cx, handle, "needs-me");
    assert_eq!(highlighted(cx).as_deref(), Some("m-handoff"), "the next press moves on");
    click(cx, handle, "needs-me");
    assert_eq!(highlighted(cx).as_deref(), Some("m-conflict"), "and wraps around");
}
