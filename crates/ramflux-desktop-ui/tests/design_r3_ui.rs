// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI integration tests of the interactions the design alignment added: the
//! identity popover (switch, window commands), the Unread chip, the people
//! table and its keyboard cursor, the dock tabs and section tools, the
//! Appearance mode control, the Tasks and Agents sections, and the status
//! banner's source alarm.

mod common;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::test::TestWindowExt as _;
use ramflux_desktop_core::{AccountId, DomainEvent, SourceStatus};
use ramflux_desktop_test::load_fixture;
use ramflux_desktop_ui::preferences::{self, ThemeChoice};
use ramflux_desktop_ui::settings::SettingsSection;
use ramflux_desktop_ui::workspace::{DockTool, WorkspaceRoute};

type Handle = gpui_kit::WindowHandle<gpui_kit::base::Root>;

fn navigate(
    cx: &mut TestAppContext,
    handle: Handle,
    shell: &gpui_kit::Entity<ramflux_desktop_ui::Shell>,
    route: WorkspaceRoute,
) {
    let workspace = workspace(cx, shell);
    in_window(cx, handle, |window, cx| {
        workspace.update(cx, |w, cx| w.navigate(route, window, cx));
    });
}

#[gpui_kit::test]
fn the_identity_popover_switches_accounts_and_the_palette_has_a_visible_entry(
    cx: &mut TestAppContext,
) {
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s09_multi_account_day");
    advance(cx, 2_000);
    click(cx, handle, "nav-account");
    assert!(exists(cx, handle, "identity-menu"), "the avatar opens the identity popover");
    for id in ["identity-palette", "identity-theme", "identity-settings"] {
        assert!(!exists(cx, handle, id), "{id}: the popover is identity only (board B01)");
    }
    click(cx, handle, "identity-switch-a-work");
    let store = store_handle(cx, &shell);
    assert_eq!(
        cx.update(|cx| store.read(cx).active_account().cloned()),
        Some(AccountId::new("a-work")),
        "Switch shows the other account"
    );
    click(cx, handle, "open-palette");
    assert!(cx.update(|cx| shell.read(cx).is_palette_open()), "the palette opens by pointer");
}

#[gpui_kit::test]
fn the_unread_chip_lists_only_unread_conversations_until_the_section_changes(
    cx: &mut TestAppContext,
) {
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    assert!(exists(cx, handle, "stack-c-team"));
    click(cx, handle, "stack-filter-unread");
    assert!(exists(cx, handle, "stack-c-lena"), "Lena has unread messages");
    assert!(!exists(cx, handle, "stack-c-team"), "a read conversation is not listed");
    navigate(cx, handle, &shell, WorkspaceRoute::Groups);
    navigate(cx, handle, &shell, WorkspaceRoute::Messages);
    assert!(exists(cx, handle, "stack-c-team"), "a new visit starts with every conversation");
}

#[gpui_kit::test]
fn the_people_table_opens_a_profile_by_pointer_and_by_keyboard(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 1_000);
    navigate(cx, handle, &shell, WorkspaceRoute::Contacts);
    assert!(exists(cx, handle, "contacts-people"), "the people table before a choice");
    click(cx, handle, "contacts-view-p-lena");
    assert!(exists(cx, handle, "contact-safety-number"), "View profile opens the profile");
    click(cx, handle, "contacts-people-entry");
    assert!(exists(cx, handle, "contacts-people"), "People returns to the table");
    // The search field's cursor moves over the table; Enter opens the row.
    click(cx, handle, "contacts-search-input");
    in_window(cx, handle, |window, cx| {
        window.press("down", cx);
        window.press("enter", cx);
    });
    assert!(exists(cx, handle, "contact-safety-number"), "Enter opens the row under the cursor");
}

#[gpui_kit::test]
fn the_dock_panel_switches_tools_by_its_tabs_and_closes(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    show_context(cx, handle, &shell, "c-team");
    click(cx, handle, "rail-objects");
    let open = |cx: &mut TestAppContext| {
        let w = workspace(cx, &shell);
        cx.update(|cx| w.read(cx).open_tool_kind())
    };
    assert_eq!(open(cx), Some(DockTool::Objects));
    assert!(exists(cx, handle, "dock-panel-objects"));
    assert!(exists(cx, handle, "dock-rail"), "the header keeps its tools (board H01)");
    click(cx, handle, "dock-tab-members");
    assert_eq!(open(cx), Some(DockTool::Members), "a tab switches the tool");
    click(cx, handle, "dock-close-members");
    assert_eq!(open(cx), None, "Close closes the dock");
    // Boards 03/06/07: a section page has no tools; they belong to a
    // conversation (ruling b).
    navigate(cx, handle, &shell, WorkspaceRoute::Contacts);
    assert!(!exists(cx, handle, "dock-rail"), "no tool row on a section page");
}

#[gpui_kit::test]
fn the_appearance_mode_control_sets_light_dark_and_follow_system(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 1_000);
    cx.update(|cx| {
        shell.update(cx, |shell, cx| shell.open_settings_at(SettingsSection::Appearance, cx))
    });
    cx.run_until_parked();
    click(cx, handle, "appearance-mode-dark");
    assert!(cx.update(|cx| cx.theme().is_dark()), "Dark paints the dark set");
    click(cx, handle, "appearance-mode-light");
    assert!(!cx.update(|cx| cx.theme().is_dark()), "Light paints the light set");
    click(cx, handle, "appearance-mode-system");
    assert_eq!(cx.update(|cx| preferences::theme_choice(cx)), ThemeChoice::System);
    assert!(!exists(cx, handle, "appearance-dark-toggle"), "one appearance control, not two");
    cx.update(preferences::install_fixed);
}

#[gpui_kit::test]
fn the_tasks_section_lists_categories_and_starts_a_proposal(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s10_ai_participant");
    advance(cx, 200);
    show_context(cx, handle, &shell, "c-launch");
    type_into(cx, handle, "composer-input", "@Orbit can you check the launch checklist?");
    click(cx, handle, "composer-send");
    advance(cx, 1_100);
    navigate(cx, handle, &shell, WorkspaceRoute::Tasks);
    for id in ["tasks-category-needs_me", "tasks-category-all", "tasks-category-running"] {
        assert!(exists(cx, handle, id), "{id}");
    }
    assert!(exists(cx, handle, "task-card-t-launch"), "the proposal waits for me");
    assert!(exists(cx, handle, "task-scope-t-launch"), "the missing scope is said, not invented");
    click(cx, handle, "task-start-t-launch");
    advance(cx, 1_400);
    let state = with_store(cx, &shell, |s| {
        s.task(&ramflux_desktop_core::TaskId::new("t-launch")).map(|t| t.state)
    });
    assert_ne!(
        state,
        Some(ramflux_desktop_core::TaskState::Proposed),
        "Confirm and start starts it"
    );
    click(cx, handle, "tasks-category-all");
    click(cx, handle, "task-open-t-launch");
    assert_eq!(
        {
            let w = workspace(cx, &shell);
            cx.update(|cx| w.read(cx).route())
        },
        WorkspaceRoute::Messages,
        "View details opens the task in its conversation"
    );
}

#[gpui_kit::test]
fn the_agents_section_opens_the_private_conversation_with_its_profile(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    click(cx, handle, "nav-agents");
    assert_eq!(
        {
            let w = workspace(cx, &shell);
            cx.update(|cx| w.read(cx).route())
        },
        WorkspaceRoute::Agents
    );
    assert!(exists(cx, handle, "agent-roster"));
    click(cx, handle, "agent-p-orbit");
    assert_eq!(
        {
            let w = workspace(cx, &shell);
            cx.update(|cx| w.read(cx).current_context().cloned())
        },
        Some(ramflux_desktop_core::ContextId::new("c-orbit"))
    );
    assert!(exists(cx, handle, "conversation-agent-profile"), "View profile in the header");
    assert!(exists(cx, handle, "composer-receiver"), "the composer names the receiver");
    click(cx, handle, "conversation-agent-profile");
    assert!(cx.update(|cx| shell.read(cx).is_settings_open()), "View profile opens its page");
}

#[gpui_kit::test]
fn the_banner_raises_the_source_alarm_only_while_the_daemon_is_not_connected(
    cx: &mut TestAppContext,
) {
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    assert!(!exists(cx, handle, "status-source-value"), "connected: no alarm");
    let mut scenario = load_fixture("s02_daily").expect("test: scenario");
    scenario.timeline.push((3_100, DomainEvent::SourceStatusChanged(SourceStatus::Disconnected)));
    let store = store_handle(cx, &shell);
    cx.update(|cx| store.update(cx, |s, cx| s.load_scenario(&scenario, cx)));
    advance(cx, 3_200);
    assert!(exists(cx, handle, "status-source-value"), "disconnected: the banner says so");
}
