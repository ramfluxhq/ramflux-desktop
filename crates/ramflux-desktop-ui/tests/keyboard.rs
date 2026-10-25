// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The whole product by keyboard only: the entry flow, switching contexts
//! (secondary-1..6, the palette), "needs me" (secondary-j), the composer,
//! approve, deny and the hold, opening and closing docks, and settings.
//! Each step asserts where keyboard focus is and what the user's keys did.

mod common;

use common::*;
use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{Entity, TestAppContext, WindowHandle};
use ramflux_desktop_core::{ContextId, GrantId, GrantState, methods};
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::confirm::HOLD_DURATION;
use ramflux_desktop_ui::route::{EntryStep, Page};
use ramflux_desktop_ui::settings::SettingsSection;
use ramflux_desktop_ui::workspace::DockTool;

type Handle = WindowHandle<gpui_kit::base::Root>;

fn press(cx: &mut TestAppContext, handle: Handle, keys: &'static str) {
    in_window(cx, handle, move |window, cx| window.press(keys, cx));
}

fn typed(cx: &mut TestAppContext, handle: Handle, text: &str) {
    let text = text.to_owned();
    in_window(cx, handle, move |window, cx| window.input(&text, cx));
}

/// Whether the element `id` (inside `scope` when given) holds keyboard focus.
fn focused(cx: &mut TestAppContext, handle: Handle, scope: Option<&str>, id: &str) -> bool {
    let (scope, id) = (scope.map(str::to_owned), id.to_owned());
    in_window(cx, handle, move |window, _| {
        let found = match scope {
            Some(scope) => window.within(scope).try_find(id),
            None => window.try_find(id),
        };
        found.and_then(|e| e.focused()) == Some(true)
    })
}

/// Presses Tab until `id` has focus; panics after `max` presses.
fn tab_to(cx: &mut TestAppContext, handle: Handle, scope: Option<&str>, id: &str, max: usize) {
    for _ in 0..max {
        if focused(cx, handle, scope, id) {
            return;
        }
        press(cx, handle, "tab");
    }
    assert!(focused(cx, handle, scope, id), "Tab never reached {id}");
}

fn current(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Option<ContextId> {
    let workspace = workspace(cx, shell);
    cx.update(|cx| workspace.read(cx).current_context().cloned())
}

fn composer_focused(cx: &mut TestAppContext, handle: Handle) -> bool {
    focused(cx, handle, None, "composer-input")
}

#[gpui_kit::test]
fn the_entry_flow_by_keyboard_confirms_a_node_and_creates_an_account(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s01_first_launch");
    advance(cx, 3_000);
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Node));
    tab_to(cx, handle, Some("node-n-harbor"), "node-confirm", 40);
    press(cx, handle, "enter");
    assert_eq!(last_body(cx, &shell, methods::NODE_CONFIRM)["node_id"], "n-harbor");
    advance(cx, 300);
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Account));
    tab_to(cx, handle, None, "account-label", 40);
    typed(cx, handle, "Personal");
    tab_to(cx, handle, None, "account-secret", 10);
    typed(cx, handle, "river stone lantern");
    tab_to(cx, handle, None, "account-create", 10);
    press(cx, handle, "enter");
    let body = last_body(cx, &shell, methods::ACCOUNT_CREATE);
    assert_eq!(body["label"], "Personal");
    assert!(!body.to_string().contains("lantern"), "the passphrase travels as a secret");
    advance(cx, 800);
    assert_eq!(page(cx, &shell), Page::Workspace, "created by keyboard alone");
}

#[gpui_kit::test]
fn contexts_switch_by_palette_pin_and_stack_keys_and_land_in_the_composer(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    assert_eq!(current(cx, &shell), Some(ContextId::new("c-team")));

    // The palette: open, search, choose; focus returns to where it was.
    press(cx, handle, "secondary-1");
    assert!(composer_focused(cx, handle), "secondary-1 lands in the composer");
    press(cx, handle, "secondary-k");
    assert!(cx.update(|cx| shell.read(cx).is_palette_open()));
    typed(cx, handle, "Lena");
    press(cx, handle, "enter");
    assert!(!cx.update(|cx| shell.read(cx).is_palette_open()));
    assert_eq!(current(cx, &shell), Some(ContextId::new("c-lena")));
    assert!(composer_focused(cx, handle), "focus returns to the composer");

    // Pin the shown context by keyboard; it joins the stack.
    press(cx, handle, "secondary-shift-p");
    advance(cx, 200);
    assert!(with_store(cx, &shell, |s| s
        .context(&ContextId::new("c-lena"))
        .is_some_and(|c| c.pinned)));

    press(cx, handle, "secondary-1");
    assert_eq!(current(cx, &shell), Some(ContextId::new("c-team")));
    assert!(composer_focused(cx, handle));
    press(cx, handle, "secondary-2");
    assert_eq!(current(cx, &shell), Some(ContextId::new("c-lena")));
    assert!(composer_focused(cx, handle));
    press(cx, handle, "secondary-6");
    assert_eq!(current(cx, &shell), Some(ContextId::new("c-lena")), "no sixth entry: no change");

    // Write and send from the composer.
    typed(cx, handle, "Lunch at noon");
    press(cx, handle, "enter");
    let body = last_body(cx, &shell, methods::SEND_MESSAGE);
    assert_eq!(
        (body["context_id"].as_str(), body["text"].as_str()),
        (Some("c-lena"), Some("Lunch at noon"))
    );

    // Escape closes the palette without choosing; focus comes back.
    press(cx, handle, "secondary-k");
    press(cx, handle, "escape");
    assert!(!cx.update(|cx| shell.read(cx).is_palette_open()));
    assert!(composer_focused(cx, handle));
}

#[gpui_kit::test]
fn needs_me_then_tab_enter_and_a_held_key_approve_a_high_risk_grant(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s03_intervention");
    advance(cx, 1_000);
    press(cx, handle, "secondary-j");
    assert!(focused(cx, handle, None, "moment-m-approve"), "needs me focuses the moment");
    tab_to(cx, handle, Some("moment-m-approve"), "approve", 6);
    press(cx, handle, "enter");
    assert!(strip_open(cx, &shell), "approve asks for the hold");
    assert!(focused(cx, handle, None, "hold-confirm"), "focus moves to the hold control");

    // A short key press is not a hold.
    press(cx, handle, "space");
    advance(cx, 1_500);
    assert!(effects(cx, &shell).is_empty() && strip_open(cx, &shell));

    // Press and hold Space: the progress shows, then the hold completes.
    in_window(cx, handle, |window, cx| window.key_down("space", false, cx));
    advance(cx, 400);
    let progress =
        cx.update(|cx| shell.read(cx).root().read(cx).confirm_strip().read(cx).progress());
    assert!(progress > 0.1 && progress < 1.0, "visible progress while held: {progress}");
    let value =
        in_window(cx, handle, |window, _| window.find("hold-confirm").label().map(str::to_owned));
    assert!(value.is_some());
    advance(cx, u64::try_from(HOLD_DURATION.as_millis()).unwrap_or(0));
    in_window(cx, handle, |window, cx| window.key_up("space", cx));
    let body = last_body(cx, &shell, methods::GRANT_APPROVE);
    assert_eq!(body["confirmed_by"]["gesture"], "Hold");
    assert!(!strip_open(cx, &shell));
    assert!(composer_focused(cx, handle), "focus returns to the composer");
    advance(cx, 1_000);
    let state = with_store(cx, &shell, |s| s.grant(&GrantId::new("g-rotate")).map(|g| g.state));
    assert_eq!(state, Some(GrantState::Approved));
}

#[gpui_kit::test]
fn escape_cancels_the_hold_and_deny_needs_no_hold(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s03_intervention");
    advance(cx, 1_000);
    press(cx, handle, "secondary-j");
    tab_to(cx, handle, Some("moment-m-approve"), "approve", 6);
    press(cx, handle, "enter");
    assert!(strip_open(cx, &shell));
    press(cx, handle, "escape");
    assert!(!strip_open(cx, &shell), "Escape cancels");
    assert!(effects(cx, &shell).is_empty(), "nothing was submitted");
    assert!(composer_focused(cx, handle));

    press(cx, handle, "secondary-j");
    tab_to(cx, handle, Some("moment-m-approve"), "deny", 8);
    press(cx, handle, "enter");
    assert_eq!(effects(cx, &shell), [methods::GRANT_DENY]);
    assert!(!strip_open(cx, &shell), "denying needs no hold");
}

#[gpui_kit::test]
fn docks_open_with_focus_and_escape_returns_to_the_composer(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    let workspace = workspace(cx, &shell);
    let open_tool = |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).open_tool_kind());
    press(cx, handle, "secondary-alt-4");
    assert_eq!(open_tool(cx), Some(DockTool::Approvals));
    assert!(focused(cx, handle, None, "dock-panel-approvals"), "focus moves into the dock");
    press(cx, handle, "escape");
    assert_eq!(open_tool(cx), None, "Escape closes the dock");
    assert!(composer_focused(cx, handle));
    press(cx, handle, "secondary-alt-1");
    assert_eq!(open_tool(cx), Some(DockTool::Objects));
    press(cx, handle, "secondary-alt-1");
    assert_eq!(open_tool(cx), None, "the same key closes it again");
    assert!(composer_focused(cx, handle));
}

#[gpui_kit::test]
fn settings_open_trap_focus_step_sections_and_close_back(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    press(cx, handle, "secondary-1");
    assert!(composer_focused(cx, handle));
    press(cx, handle, "secondary-,");
    assert!(cx.update(|cx| shell.read(cx).is_settings_open()));
    assert!(focused(cx, handle, None, "settings"), "focus moves into settings");
    let section =
        |cx: &mut TestAppContext| cx.update(|cx| shell.read(cx).settings().read(cx).section());
    press(cx, handle, "ctrl-tab");
    assert_eq!(section(cx), SettingsSection::Backup);
    press(cx, handle, "ctrl-shift-tab");
    press(cx, handle, "ctrl-shift-tab");
    assert_eq!(section(cx), SettingsSection::Appearance, "wraps around");
    for _ in 0..12 {
        press(cx, handle, "tab");
        let inside = in_window(cx, handle, |window, _| {
            window.elements().iter().any(|e| {
                e.focused() == Some(true) && e.path().iter().any(|id| id.to_string() == "settings")
            })
        });
        assert!(inside, "Tab stays inside the settings overlay");
    }
    press(cx, handle, "escape");
    assert!(!cx.update(|cx| shell.read(cx).is_settings_open()));
    assert!(composer_focused(cx, handle), "focus returns to where it was");
}
