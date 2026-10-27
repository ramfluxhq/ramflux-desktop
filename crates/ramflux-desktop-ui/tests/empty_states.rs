// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Loading versus loaded-and-empty: every list shows a spinner only until
//! the source listed its collection, then an explicit empty state (H1).

mod common;

use common::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowHandle};
use ramflux_desktop_core::{
    Account, AccountId, AccountKind, DomainEvent, Listing, LockState, SourceStatus, Timestamp,
};
use ramflux_desktop_test::Scenario;
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::i18n::tr;
use ramflux_desktop_ui::route::{EntryStep, Page};
use ramflux_desktop_ui::workspace::DockTool;

type Handle = WindowHandle<gpui_kit::base::Root>;

/// An unlocked account with no contexts; the collections are listed at 500 ms.
fn empty_account() -> Scenario {
    let account = Account {
        id: AccountId::new("a-empty"),
        label: "Empty".to_owned(),
        kind: AccountKind::HumanInteractive,
        node_id: None,
        device_id: None,
        lock_state: LockState::Unlocked,
        self_participant_id: None,
        secure_sessions: ramflux_desktop_core::SecureSessions::Ready,
        networking: ramflux_desktop_core::Networking::Live,
    };
    Scenario {
        name: "empty_account".to_owned(),
        description: "An account whose collections are listed late and empty".to_owned(),
        epoch: Timestamp(1_790_812_800_000),
        timeline: vec![
            (0, DomainEvent::SourceStatusChanged(SourceStatus::Connected)),
            (0, DomainEvent::AccountUpdated(account)),
            (
                500,
                DomainEvent::ListingsLoaded(vec![
                    Listing::Contexts,
                    Listing::Contacts,
                    Listing::Objects,
                    Listing::Approvals,
                ]),
            ),
        ],
        responses: Vec::new(),
        companions: Vec::new(),
    }
}

fn load(cx: &mut TestAppContext, shell: &Entity<Shell>, scenario: &Scenario) {
    let store = store_handle(cx, shell);
    cx.update(|cx| store.update(cx, |store, cx| store.load_scenario(scenario, cx)));
    cx.run_until_parked();
}

fn label(cx: &mut TestAppContext, handle: Handle, id: &'static str) -> Option<String> {
    in_window(cx, handle, move |window, _| {
        window.try_find(id).and_then(|found| found.label().map(str::to_owned))
    })
}

fn open_tool(cx: &mut TestAppContext, handle: Handle, shell: &Entity<Shell>, tool: DockTool) {
    let workspace = workspace(cx, shell);
    cx.update_window(handle.into(), |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_tool(tool, window, cx));
    })
    .expect("window");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_node_step_waits_only_until_nodes_are_listed(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s01_first_launch");
    advance(cx, 1_600);
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Node));
    assert_eq!(
        label(cx, handle, "entry-node-loading"),
        Some(tr("entry.node.waiting").to_string()),
        "the daemon answered but has not listed nodes yet"
    );
    advance(cx, 600);
    assert!(!exists(cx, handle, "entry-node-loading"), "nodes listed: no spinner");
    assert!(!exists(cx, handle, "entry-node-empty"), "and not empty either");
}

#[gpui_kit::test]
fn lists_show_loading_then_an_explicit_empty_state(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s02_daily");
    load(cx, &shell, &empty_account());
    advance(cx, 100);
    assert_eq!(page(cx, &shell), Page::Workspace);
    let loading = Some(tr("list.loading").to_string());
    assert_eq!(label(cx, handle, "stack-list-state"), loading);
    assert_eq!(label(cx, handle, "conversation-list-state"), loading);
    open_tool(cx, handle, &shell, DockTool::Contacts);
    assert_eq!(label(cx, handle, "contacts-list-state"), loading);

    advance(cx, 500);
    assert_eq!(label(cx, handle, "stack-list-state"), Some(tr("stack.empty").to_string()));
    assert_eq!(
        label(cx, handle, "conversation-list-state"),
        Some(tr("conversation.no_contexts").to_string())
    );
    assert_eq!(
        label(cx, handle, "contacts-list-state"),
        Some(tr("dock.contacts.empty").to_string())
    );
    open_tool(cx, handle, &shell, DockTool::Objects);
    assert_eq!(label(cx, handle, "objects-list-state"), Some(tr("dock.objects.empty").to_string()));
    open_tool(cx, handle, &shell, DockTool::Approvals);
    assert_eq!(
        label(cx, handle, "approvals-list-state"),
        Some(tr("dock.approvals.none_pending").to_string())
    );
}
