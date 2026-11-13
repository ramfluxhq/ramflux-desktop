// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Production shell routes, search, focus, geometry and confirmation ownership.

mod common;
use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode};
use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
use ramflux_desktop_core::{ContextId, DomainEvent, methods};
use ramflux_desktop_test::load_fixture;
use ramflux_desktop_ui::settings::SettingsSection;
use ramflux_desktop_ui::workspace::WorkspaceRoute;

#[gpui_kit::test]
fn full_directory_search_reaches_unpinned_rows_and_enter_focuses_composer(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s02_daily");
    let mut scenario = load_fixture("s02_daily").expect("test: source scenario");
    const EXTRA_CONTEXTS: usize = 12;
    for ix in 0..EXTRA_CONTEXTS {
        let context=serde_json::from_value(serde_json::json!({"id":format!("beyond-{ix:02}"),"title":format!("Beyond {ix:02}"),"kind":"Direct"})).expect("test: source context");
        scenario.timeline.push((0, DomainEvent::ContextUpserted(context)));
    }
    scenario.timeline.sort_by_key(|(at, _)| *at);
    let store = store_handle(cx, &shell);
    cx.update(|cx| store.update(cx, |store, cx| store.load_scenario(&scenario, cx)));
    advance(cx, 3_000);
    in_window(cx, handle, |window, cx| window.press("secondary-f", cx));
    assert!(
        in_window(cx, handle, |window, _| window.find("conversation-search").focused()
            == Some(true)),
        "window search command must reach the retained list input"
    );
    in_window(cx, handle, |window, cx| window.input("Beyond 11", cx));
    assert!(
        exists(cx, handle, "stack-beyond-11"),
        "full search must include conversations beyond the old six-entry stack"
    );
    in_window(cx, handle, |window, cx| window.press("enter", cx));
    let workspace = workspace(cx, &shell);
    assert_eq!(
        cx.update(|cx| workspace.read(cx).current_context().cloned()),
        Some(ContextId::new("beyond-11")),
        "Enter must select the real source conversation"
    );
    assert!(
        in_window(cx, handle, |window, _| window.find("composer-input").focused() == Some(true)),
        "selection must hand focus to the conversation composer"
    );
    in_window(cx, handle, |window, cx| {
        window.press("secondary-f", cx);
        window.press("secondary-a", cx);
        window.input("Beyond", cx);
    });
    in_window(cx, handle, |window, cx| window.press("down", cx));
    in_window(cx, handle, |window, cx| window.press("enter", cx));
    assert_eq!(
        cx.update(|cx| workspace.read(cx).current_context().cloned()),
        Some(ContextId::new("beyond-01")),
        "Down and Enter must navigate the filtered model through real input"
    );
    assert!(
        effects(cx, &shell).is_empty(),
        "search and navigation must never submit business effects"
    );
}

#[gpui_kit::test]
fn rail_routes_to_existing_features_and_settings_restores_focus(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    click(cx, handle, "nav-groups");
    let workspace = workspace(cx, &shell);
    assert_eq!(
        cx.update(|cx| workspace.read(cx).route()),
        WorkspaceRoute::Groups,
        "group rail item must select the group projection"
    );
    assert!(
        !exists(cx, handle, "stack-c-lena"),
        "group projection cannot include direct conversations"
    );
    click(cx, handle, "nav-messages");
    click(cx, handle, "strangers-entry");
    assert_eq!(
        cx.update(|cx| workspace.read(cx).route()),
        WorkspaceRoute::Strangers,
        "stranger entry must route to requests without accepting a sender"
    );
    assert!(
        !effects(cx, &shell).iter().any(|method| method == methods::CONTACT_REQUEST_ACCEPT),
        "opening strangers cannot imply acceptance"
    );
    click(cx, handle, "nav-agents");
    assert!(
        cx.update(|cx| shell.read(cx).is_settings_open()),
        "Agents must open its existing feature"
    );
    assert_eq!(
        cx.update(|cx| shell.read(cx).settings().read(cx).section()),
        SettingsSection::Agents,
        "Agents cannot open an unrelated settings section"
    );
    in_window(cx, handle, |window, cx| window.press("escape", cx));
    assert!(!cx.update(|cx| shell.read(cx).is_settings_open()), "Escape must dismiss settings");
    assert!(
        in_window(cx, handle, |window, _| window.find("nav-agents").focused() == Some(true)),
        "dismissal must restore focus to the rail trigger"
    );
}

#[gpui_kit::test]
fn shell_geometry_and_compact_back_work_at_all_requested_widths_and_themes(
    cx: &mut TestAppContext,
) {
    let _locale = locale_guard();
    for width in [640., 800., 1024., 1440.] {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let (handle, _shell) = open_width(cx, "s02_daily", width);
            advance(cx, 3_000);
            cx.update(|cx| Theme::change(mode, None, cx));
            cx.run_until_parked();
            in_window(cx, handle, |window, cx| {
                let rail = window.find("navigation-rail").bounds();
                let list = window.find("section-list").bounds();
                assert!(
                    (f32::from(rail.size.width) - 68.).abs() < 1.,
                    "RF rail must resolve to its token width at {width}"
                );
                assert!(
                    (f32::from(list.origin.x) - f32::from(rail.right())).abs() < 1.,
                    "list must align exactly with the rail at {width}"
                );
                assert!(
                    f32::from(list.right()) <= width + 1.,
                    "list must remain inside the viewport at {width}"
                );
                assert_eq!(
                    cx.theme().is_dark(),
                    mode == ThemeMode::Dark,
                    "registered RF themes must survive mode switches"
                );
            });
            if width < 800. {
                assert!(
                    !exists(cx, handle, "composer-input"),
                    "compact list owns its viewport before selection"
                );
                click(cx, handle, "stack-c-lena");
                assert!(
                    exists(cx, handle, "back-to-list"),
                    "compact detail needs a visible return path"
                );
                click(cx, handle, "back-to-list");
                assert!(
                    exists(cx, handle, "conversation-search"),
                    "Back must restore the retained list"
                );
                assert!(
                    in_window(cx, handle, |window, _| window.find("conversation-search").focused()
                        == Some(true)),
                    "Back must restore useful keyboard focus"
                );
            } else {
                assert!(
                    exists(cx, handle, "composer-input"),
                    "split widths must retain the conversation detail"
                );
            }
        }
    }
}

#[gpui_kit::test]
fn navigation_cancels_an_unseen_confirmation_instead_of_completing_it(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s03_intervention");
    advance(cx, 1_000);
    in_window(cx, handle, |window, cx| window.press("secondary-j", cx));
    click_in(cx, handle, "moment-m-approve", "approve");
    assert!(strip_open(cx, &shell), "source high-risk grant must first ask for the existing hold");
    click(cx, handle, "nav-groups");
    assert!(!strip_open(cx, &shell), "navigation must cancel the old reviewed gesture");
    assert!(
        effects(cx, &shell).is_empty(),
        "navigation cannot mint or submit a human confirmation"
    );
}

#[gpui_kit::test]
fn large_directory_updates_only_changed_rows_and_keeps_cursor_identity(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    const DIRECTORY_SIZE: usize = 1000;
    let (handle, shell) = open(cx, "s02_daily");
    let mut scenario = load_fixture("s02_daily").expect("test: source scenario");
    for ix in 0..DIRECTORY_SIZE {
        let context = serde_json::from_value(serde_json::json!({"id":format!("scale-{ix:04}"),"title":format!("Scale {ix:04}"),"kind":"Direct"})).expect("test: source context");
        scenario.timeline.push((0, DomainEvent::ContextUpserted(context)));
    }
    let changed = serde_json::from_value(
        serde_json::json!({"id":"scale-0001","title":"Scale 0001","kind":"Direct","pinned":true}),
    )
    .expect("test: reordered context");
    scenario.timeline.push((4000, DomainEvent::ContextUpserted(changed)));
    scenario.timeline.sort_by_key(|(at, _)| *at);
    let store = store_handle(cx, &shell);
    cx.update(|cx| store.update(cx, |s, cx| s.load_scenario(&scenario, cx)));
    advance(cx, 3000);
    in_window(cx, handle, |w, cx| {
        w.press("secondary-f", cx);
        w.input("Scale", cx);
    });
    in_window(cx, handle, |w, cx| w.press("down", cx));
    let workspace = workspace(cx, &shell);
    let list = cx.update(|cx| workspace.read(cx).conversation_list().clone());
    let before = cx.update(|cx| list.read(cx).projected_rows());
    assert_eq!(
        cx.update(|cx| list.read(cx).entry_ids(store.read(cx).store()).len()),
        DIRECTORY_SIZE,
        "the retained model must contain the full directory"
    );
    in_window(cx, handle, |w, _| {
        assert!(w.try_find("stack-scale-0999").is_none(), "offscreen rows must remain lazy");
        assert!(
            w.find_all_by_role(gpui_kit::Role::Button).len() < 100,
            "a thousand-row model must render only a viewport"
        );
    });
    advance(cx, 1000);
    assert_eq!(
        cx.update(|cx| list.read(cx).projected_rows()) - before,
        1,
        "one context delta must project exactly one row"
    );
    in_window(cx, handle, |w, cx| w.press("enter", cx));
    assert_eq!(
        cx.update(|cx| workspace.read(cx).current_context().cloned()),
        Some(ContextId::new("scale-0001")),
        "reordering must preserve the active domain ID"
    );
}

#[gpui_kit::test]
fn resize_and_context_selection_cancel_pending_and_in_progress_holds(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (handle, shell) = open_width(cx, "s03_intervention", 1024.);
    advance(cx, 1000);
    // Focus the request without selecting a list row: this keeps the wide list
    // preference, reproducing a resize that hides detail.
    in_window(cx, handle, |w, cx| w.press("secondary-j", cx));
    click_in(cx, handle, "moment-m-approve", "approve");
    assert!(strip_open(cx, &shell), "the source grant must first request confirmation");
    let center = in_window(cx, handle, |w, cx| {
        let center = w.find("hold-confirm").bounds().center();
        w.pointer_down(center, gpui_kit::MouseButton::Left, cx);
        center
    });
    cx.simulate_window_resize(
        handle.into(),
        gpui_kit::size(gpui_kit::px(640.), gpui_kit::px(860.)),
    );
    cx.run_until_parked();
    in_window(cx, handle, |_, _| {});
    assert!(
        !strip_open(cx, &shell),
        "hiding reviewed facts must cancel even an in-progress gesture"
    );
    advance(cx, 2000);
    in_window(cx, handle, |w, cx| w.pointer_up(center, gpui_kit::MouseButton::Left, cx));
    assert!(effects(cx, &shell).is_empty(), "a hidden hold must never complete later");
    in_window(cx, handle, |w, cx| w.press("secondary-1", cx));
    click_in(cx, handle, "moment-m-approve", "approve");
    assert!(strip_open(cx, &shell), "the request must still be actionable after cancellation");
    let workspace = workspace(cx, &shell);
    in_window(cx, handle, move |w, cx| {
        workspace.update(cx, |s, cx| s.select_context(Some(ContextId::new("other")), w, cx))
    });
    assert!(
        !strip_open(cx, &shell),
        "selecting another context must cancel its predecessor's confirmation"
    );
    assert!(effects(cx, &shell).is_empty(), "context selection is never consent");
}

#[gpui_kit::test]
fn bounded_previews_tasks_unknown_history_and_compact_return_are_visible(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (handle, shell) = open_width(cx, "s02_daily", 640.);
    let mut scenario = load_fixture("s02_daily").expect("test: source scenario");
    const LARGE_BODY_BYTES: usize = 1_048_576;
    for (_, event) in &mut scenario.timeline {
        if let DomainEvent::MomentAdded(moment) = event
            && let ramflux_desktop_core::MomentKind::Message(payload) = &mut moment.kind
        {
            payload.text = format!("مرحبا\u{202e} first line\n{}", "x".repeat(LARGE_BODY_BYTES));
        }
    }
    let unknown = serde_json::from_value(serde_json::json!({"id":"unloaded-task","title":"Unloaded","kind":"Direct","last_activity":2000000})).expect("test: unloaded directory row");
    scenario.timeline.push((0, DomainEvent::ContextUpserted(unknown)));
    scenario.timeline.sort_by_key(|(at, _)| *at);
    let store = store_handle(cx, &shell);
    cx.update(|cx| store.update(cx, |s, cx| s.load_scenario(&scenario, cx)));
    advance(cx, 3000);
    in_window(cx, handle, |w, _| {
        assert!(
            (f32::from(w.find("stack-c-team").bounds().size.height) - 64.).abs() < 1.,
            "multiline hostile text must not change fixed row height"
        );
    });
    click(cx, handle, "nav-tasks");
    assert!(
        exists(cx, handle, "tasks-history-notice"),
        "the route must disclose incomplete task history"
    );
    assert!(
        exists(cx, handle, "stack-unloaded-task"),
        "unloaded task contexts cannot silently disappear"
    );
    click(cx, handle, "stack-unloaded-task");
    in_window(cx, handle, |w, cx| w.press("secondary-[", cx));
    assert!(
        exists(cx, handle, "conversation-search"),
        "the Back shortcut must work with composer focused"
    );
}
