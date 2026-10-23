// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI integration tests: the production window root in a headless window,
//! driven by real pointer and keyboard input and the scenario clock.

mod common;

use common::*;
use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AppContext as _, MouseButton, TestAppContext};
use ramflux_desktop_core::{
    ContextId, GrantId, GrantState, IntentKind, Lifecycle, MomentId, ParticipantId,
};
use ramflux_desktop_ui::app::{InspectorEntry, UiOutcome};
use ramflux_desktop_ui::confirm::HOLD_DURATION;
use ramflux_desktop_ui::route::{EntryStep, Page};

fn page(cx: &mut TestAppContext, shell: &gpui_kit::Entity<ramflux_desktop_ui::Shell>) -> Page {
    cx.update(|cx| shell.read(cx).page())
}

fn grant_state(
    cx: &mut TestAppContext,
    shell: &gpui_kit::Entity<ramflux_desktop_ui::Shell>,
) -> GrantState {
    with_store(cx, shell, |store| store.grant(&GrantId::new("g-rotate")).expect("grant").state)
}

/// Holds the confirm control for longer than [`HOLD_DURATION`].
fn hold_confirm(cx: &mut TestAppContext, handle: gpui_kit::WindowHandle<gpui_kit::base::Root>) {
    let center = in_window(cx, handle, |window, cx| {
        let center = window.find("hold-confirm").bounds().center();
        window.pointer_down(center, MouseButton::Left, cx);
        center
    });
    advance(cx, u64::try_from(HOLD_DURATION.as_millis()).unwrap_or(0) + 200);
    in_window(cx, handle, |window, cx| window.pointer_up(center, MouseButton::Left, cx));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn s01_entry_flow_reaches_the_workspace(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s01_first_launch");
    advance(cx, 100);
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Daemon));
    advance(cx, 2_500);
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Node), "node joined from the invite");
    advance(cx, 2_000);
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Account), "account created, locked");

    in_window(cx, handle, |window, cx| {
        window.click("passphrase", cx);
        window.input("river stone lantern", cx);
        window.press("enter", cx);
    });
    cx.run_until_parked();

    assert_eq!(effects(cx, &shell), ["Unlock"]);
    let entry = cx.update(|cx| shell.read(cx).entry().clone());
    let typed = cx.update(|cx| entry.read(cx).passphrase().read(cx).value());
    assert!(typed.is_empty(), "the secret leaves the input on submit");
    let inspector = cx.update(|cx| format!("{:?}", shell.read(cx).store().read(cx).inspector()));
    assert!(!inspector.contains("river"), "the inspector never sees the secret");

    advance(cx, 500);
    assert_eq!(page(cx, &shell), Page::Workspace);
    let workspace = workspace(cx, &shell);
    let current = cx.update(|cx| workspace.read(cx).current_context().cloned());
    assert_eq!(current, Some(ContextId::new("c-notes")));
    let rows = cx.update(|cx| workspace.read(cx).conversation().read(cx).row_ids());
    assert_eq!(rows, [MomentId::new("m-welcome")]);
}

#[gpui_kit::test]
fn s02_send_message_and_switch_context(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    assert_eq!(page(cx, &shell), Page::Workspace);
    let workspace = workspace(cx, &shell);
    let current =
        |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).current_context().cloned());
    assert_eq!(current(cx), Some(ContextId::new("c-team")), "the pinned context opens first");

    // Context switch through the "switch context" menu.
    in_window(cx, handle, |window, cx| window.click("stack-switch", cx));
    cx.run_until_parked();
    in_window(cx, handle, |window, cx| {
        let item = window.find_by_label("Lena Hart").bounds().center();
        window.pointer_down(item, MouseButton::Left, cx);
        window.pointer_up(item, MouseButton::Left, cx);
    });
    cx.run_until_parked();
    assert_eq!(current(cx), Some(ContextId::new("c-lena")));

    // Composer send -> SendMessage effect -> mock response -> row appears.
    in_window(cx, handle, |window, cx| {
        window.click("composer-input", cx);
        window.input("12:30 works", cx);
        window.click("composer-send", cx);
    });
    cx.run_until_parked();
    assert_eq!(effects(cx, &shell), ["message.send"]);
    advance(cx, 100);
    let rows = cx.update(|cx| workspace.read(cx).conversation().read(cx).row_ids());
    assert_eq!(rows.last(), Some(&MomentId::new("m-lena-reply")));
    in_window(cx, handle, |window, _| {
        assert!(window.find("composer-input").value().unwrap_or_default().is_empty());
    });

    // Back to the pinned context through its stack entry.
    in_window(cx, handle, |window, cx| window.click("stack-c-team", cx));
    cx.run_until_parked();
    assert_eq!(current(cx), Some(ContextId::new("c-team")));
}

#[gpui_kit::test]
fn s03_approval_opens_the_strip_and_a_hold_confirms(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s03_intervention");
    advance(cx, 1_000);
    let workspace = workspace(cx, &shell);

    in_window(cx, handle, |window, cx| window.within("moment-m-approve").click("approve", cx));
    cx.run_until_parked();
    let strip = cx.update(|cx| workspace.read(cx).confirm_strip().clone());
    assert!(cx.update(|cx| strip.read(cx).is_open()), "high risk opens the inline strip");
    assert!(effects(cx, &shell).is_empty(), "nothing reaches the source before the hold");

    // A click is not a hold.
    in_window(cx, handle, |window, cx| window.click("hold-confirm", cx));
    advance(cx, 2_000);
    assert_eq!(grant_state(cx, &shell), GrantState::Pending);
    assert!(cx.update(|cx| strip.read(cx).is_open()));

    hold_confirm(cx, handle);
    assert_eq!(effects(cx, &shell), ["grant.approve"]);
    assert!(!cx.update(|cx| strip.read(cx).is_open()));
    advance(cx, 1_000);

    assert_eq!(grant_state(cx, &shell), GrantState::Approved);
    let lifecycle =
        with_store(cx, &shell, |s| s.moment(&MomentId::new("m-approve")).map(|m| m.lifecycle));
    assert_eq!(lifecycle, Some(Lifecycle::Done));
    let rows = cx.update(|cx| workspace.read(cx).conversation().read(cx).row_ids());
    assert!(rows.contains(&MomentId::new("m-result")), "the result returns to the conversation");
}

#[gpui_kit::test]
fn s03_agent_approval_never_bypasses_the_strip(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s03_intervention");
    advance(cx, 1_000);
    let workspace = workspace(cx, &shell);

    cx.update_window(handle.into(), |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            let kind = IntentKind::Approve { grant_id: GrantId::new("g-rotate"), confirmed: true };
            workspace.submit_agent_intent(ParticipantId::new("p-orbit"), kind, window, cx);
        });
    })
    .expect("window");
    cx.run_until_parked();

    let strip = cx.update(|cx| workspace.read(cx).confirm_strip().clone());
    assert!(cx.update(|cx| strip.read(cx).is_open()), "an agent's claimed confirmation is ignored");
    advance(cx, 2_000);
    assert!(effects(cx, &shell).is_empty());
    assert_eq!(grant_state(cx, &shell), GrantState::Pending);

    hold_confirm(cx, handle);
    advance(cx, 500);
    assert_eq!(effects(cx, &shell), ["grant.approve"]);
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
    in_window(cx, handle, |window, cx| window.within("moment-m-approve").click("details", cx));
    cx.run_until_parked();
    assert_eq!(root_intents(cx), before, "an expand click produces no intent");
    let expanded = cx.update(|cx| {
        workspace.read(cx).conversation().read(cx).is_expanded(&MomentId::new("m-approve"))
    });
    assert!(expanded, "the moment level consumed the expand click");
    let consumed = cx.update(|cx| {
        store.read(cx).inspector().entries().any(|entry| {
            matches!(entry, InspectorEntry::UiEvent { outcome: UiOutcome::Consumed { at }, .. }
                if at == "Moment(m-approve)")
        })
    });
    assert!(consumed);

    in_window(cx, handle, |window, cx| window.within("moment-m-approve").click("approve", cx));
    cx.run_until_parked();
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
    cx.run_until_parked();
    assert!(cx.update(|cx| shell.read(cx).is_dev_panel_visible()));
    advance(cx, 3_000);
    let snapshot = |cx: &mut TestAppContext| with_store(cx, &shell, |s| format!("{s:?}"));
    let before = snapshot(cx);
    let clock = cx.update(|cx| shell.read(cx).store().read(cx).session().map(|s| s.clock_ms()));
    let clock = clock.expect("session");

    in_window(cx, handle, |window, cx| window.click("dev-restart", cx));
    cx.run_until_parked();
    let reset_seq = with_store(cx, &shell, |s| s.last_seq());
    assert!(reset_seq < 10, "seek to t=0 keeps only the events at offset 0");
    assert_ne!(snapshot(cx), before);

    let dev = cx.update(|cx| shell.read(cx).dev_panel().cloned()).expect("debug build");
    cx.update(|cx| dev.update(cx, |dev, cx| dev.seek(clock, cx)));
    cx.run_until_parked();
    assert_eq!(snapshot(cx), before, "replay reaches the same state");
    let workspace = workspace(cx, &shell);
    let rows = cx.update(|cx| workspace.read(cx).conversation().read(cx).row_ids().len());
    assert!(rows > 0, "views refreshed after the reset");
}

#[gpui_kit::test]
fn go_to_needs_me_focuses_the_oldest_attention_item(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s07_conflict_privacy_handoff");
    advance(cx, 1_000);
    in_window(cx, handle, |window, cx| window.click("needs-me", cx));
    cx.run_until_parked();
    let workspace = workspace(cx, &shell);
    let highlighted =
        cx.update(|cx| workspace.read(cx).conversation().read(cx).highlighted().cloned());
    assert_eq!(highlighted, Some(MomentId::new("m-conflict")));
}

#[gpui_kit::test]
fn background_accounts_only_count_attention(cx: &mut TestAppContext) {
    let (_handle, shell) = open(cx, "s02_daily");
    let store = store_handle(cx, &shell);
    let ops = ramflux_desktop_core::AccountId::new("a-ops");
    cx.update(|cx| {
        let scenario = ramflux_desktop_test::load_fixture("s03_intervention").expect("s03");
        store.update(cx, |store, cx| store.add_background_account(ops.clone(), &scenario, cx));
    });
    advance(cx, 1_500);
    let (active_contexts, ops_attention) = cx.update(|cx| {
        let handle = store.read(cx);
        (handle.store().contexts().count(), handle.needs_me_count(&ops))
    });
    assert_eq!(active_contexts, 3, "the active account still renders s02");
    assert_eq!(ops_attention, 1, "the background account only reports attention");

    cx.update(|cx| store.update(cx, |store, cx| store.set_active_account(&ops, cx)));
    cx.run_until_parked();
    let workspace = workspace(cx, &shell);
    let current = cx.update(|cx| workspace.read(cx).current_context().cloned());
    assert_eq!(current, Some(ContextId::new("c-ops")), "switching accounts switches stores");
}
