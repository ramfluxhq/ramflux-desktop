// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI integration tests of the command palette (search and actions) and of
//! multiple accounts (background attention, switching, resolving).

mod common;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use ramflux_desktop_core::{AccountId, ContextId, methods};
use ramflux_desktop_ui::actions::OpenPalette;
use ramflux_desktop_ui::app::InspectorEntry;
use ramflux_desktop_ui::i18n::tr;

fn open_palette(cx: &mut TestAppContext, handle: gpui_kit::WindowHandle<gpui_kit::base::Root>) {
    in_window(cx, handle, |window, cx| window.dispatch_action(Box::new(OpenPalette), cx));
}

#[gpui_kit::test]
fn the_palette_lists_every_context_and_navigates(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    open_palette(cx, handle);
    assert!(cx.update(|cx| shell.read(cx).is_palette_open()));
    let palette = cx.update(|cx| shell.read(cx).palette().clone());
    let labels = cx.update(|cx| palette.read(cx).labels());
    for title in ["Lena Hart", "Design team", "Orbit"] {
        assert!(labels.iter().any(|l| l == title), "{title} is reachable from the palette");
    }
    assert!(labels.iter().any(|l| l.contains("lunch")), "history is searchable");
    in_window(cx, handle, |window, cx| window.input("Lena", cx));
    in_window(cx, handle, |window, cx| window.press("enter", cx));
    assert!(!cx.update(|cx| shell.read(cx).is_palette_open()), "choosing closes the palette");
    let workspace = workspace(cx, &shell);
    let current = cx.update(|cx| workspace.read(cx).current_context().cloned());
    assert_eq!(current, Some(ContextId::new("c-lena")));
    assert!(effects(cx, &shell).is_empty(), "navigation is not an intent");
}

#[gpui_kit::test]
fn palette_actions_become_intents_through_the_root(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    open_palette(cx, handle);
    let query = tr("conversation.mute").to_string();
    in_window(cx, handle, move |window, cx| window.input(&query, cx));
    in_window(cx, handle, |window, cx| window.press("enter", cx));
    let body = last_body(cx, &shell, methods::CONTEXT_MUTE);
    assert_eq!(body["context_id"], "c-team");
    assert_eq!(body["muted"], true);
    advance(cx, 100);
    assert_eq!(
        with_store(cx, &shell, |s| s.context(&ContextId::new("c-team")).map(|c| c.muted)),
        Some(true)
    );
    assert!(exists(cx, handle, "header-muted"), "the header shows the muted state");
    let store = store_handle(cx, &shell);
    let path = cx.update(|cx| {
        store.read(cx).inspector().root_intents().find_map(|e| match e {
            InspectorEntry::UiEvent { path, kind: "MuteToggled", .. } => Some(path.clone()),
            _ => None,
        })
    });
    assert_eq!(path.as_deref(), Some("Workspace > Panel(palette) > Context(c-team)"));
}

#[gpui_kit::test]
fn background_attention_is_reached_across_accounts_and_resolved(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s09_multi_account_day");
    advance(cx, 2_000);
    let store = store_handle(cx, &shell);
    let work = AccountId::new("a-work");
    let (active, background) = cx.update(|cx| {
        let h = store.read(cx);
        (h.active_account().cloned(), h.needs_me_count(&work))
    });
    assert_eq!(active, Some(AccountId::new("a-personal")));
    assert_eq!(background, 1, "the work account only reports its attention");
    assert!(exists(cx, handle, "status-background-attention"), "the strip shows it");
    assert!(with_store(cx, &shell, |s| s.context(&ContextId::new("c-release")).is_none()));

    let workspace = workspace(cx, &shell);
    let highlighted = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            workspace.read(cx).conversation().read(cx).highlighted().map(ToString::to_string)
        })
    };
    click(cx, handle, "needs-me");
    assert_eq!(highlighted(cx).as_deref(), Some("m-ines"), "the active account first");
    click(cx, handle, "needs-me");
    let active = cx.update(|cx| store.read(cx).active_account().cloned());
    assert_eq!(active, Some(work.clone()), "the next item switched accounts");
    assert_eq!(highlighted(cx).as_deref(), Some("m-deploy"));

    click_in(cx, handle, "moment-m-deploy", "approve");
    assert_eq!(
        last_body(cx, &shell, methods::GRANT_APPROVE)["grant_id"],
        "g-deploy",
        "sent by the work session"
    );
    advance(cx, 800);
    assert_eq!(cx.update(|cx| store.read(cx).needs_me_count(&work)), 0, "resolved");
    assert!(rows(cx, &shell).contains(&"m-deployed".to_owned()));

    click(cx, handle, "needs-me");
    let active = cx.update(|cx| store.read(cx).active_account().cloned());
    assert_eq!(active, Some(AccountId::new("a-personal")), "back to what is left");
}
