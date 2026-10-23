// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI integration tests, one per dock, plus summoned versus pinned docks.
//! Each asserts the intents and effects its actions produce.

mod common;

use common::*;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{MouseButton, TestAppContext};
use ramflux_desktop_core::{ContextId, GrantId, GrantState, MomentId, methods};
use ramflux_desktop_ui::workspace::DockTool;

/// Clicks the menu item labeled `label` (popup menus are labeled rows).
fn click_label(
    cx: &mut TestAppContext,
    handle: gpui_kit::WindowHandle<gpui_kit::base::Root>,
    label: &str,
) {
    let label = label.to_owned();
    in_window(cx, handle, move |window, cx| {
        let at = window.find_by_label(&label).bounds().center();
        gpui_kit::test::TestEventExt::pointer_down(window, at, MouseButton::Left, cx);
        gpui_kit::test::TestEventExt::pointer_up(window, at, MouseButton::Left, cx);
    });
}

#[gpui_kit::test]
fn objects_dock_lists_transfers_and_pulls(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    click(cx, handle, "rail-objects");
    assert!(exists(cx, handle, "dock-content-objects"));
    let workspace = workspace(cx, &shell);
    let ids = cx.update(|cx| {
        let dock = workspace.read(cx).objects_dock().clone();
        dock.read(cx).object_ids(cx)
    });
    assert_eq!(ids, ["o-floorplan", "o-moodboard", "o-specs"]);
    click(cx, handle, "objects-pull-o-specs");
    let body = last_body(cx, &shell, methods::OBJECT_PULL);
    assert_eq!(body["object_id"], "o-specs");
}

#[gpui_kit::test]
fn contacts_dock_answers_requests_and_verifies(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s04_safety");
    advance(cx, 1_000);
    click(cx, handle, "rail-contacts");
    assert!(exists(cx, handle, "dock-content-contacts"));
    in_window(cx, handle, |window, cx| {
        window.within("contacts-request-m-noor").click("accept", cx)
    });
    assert_eq!(last_body(cx, &shell, methods::CONTACT_ACCEPT)["moment_id"], "m-noor");
    in_window(cx, handle, |window, cx| window.within("contacts-p-mira").click("verify", cx));
    assert_eq!(last_body(cx, &shell, methods::SAFETY_VERIFY)["participant_id"], "p-mira");
    advance(cx, 200);
    let safety = with_store(cx, &shell, |s| s.participant(&"p-mira".into()).map(|p| p.safety));
    assert_eq!(safety, Some(ramflux_desktop_core::SafetyState::Verified));
}

#[gpui_kit::test]
fn members_dock_kicks_through_the_hold_strip(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s05_group_governance");
    advance(cx, 500);
    click(cx, handle, "rail-members");
    assert!(exists(cx, handle, "dock-content-members"));

    in_window(cx, handle, |window, cx| window.within("member-add-p-mira").click("add", cx));
    let body = last_body(cx, &shell, methods::GROUP_ADD_MEMBER);
    assert_eq!(body["participant_id"], "p-mira", "adding a contact is low risk: no hold");
    advance(cx, 1_200);

    in_window(cx, handle, |window, cx| window.within("member-p-tomas").click("member-actions", cx));
    click_label(cx, handle, &ramflux_desktop_ui::i18n::tr("dock.members.kick"));
    assert!(strip_open(cx, &shell), "kicking needs the hold");
    assert!(!effects(cx, &shell).contains(&methods::GROUP_KICK.to_owned()));
    hold_confirm(cx, handle);
    let body = last_body(cx, &shell, methods::GROUP_KICK);
    assert_eq!(body["participant_id"], "p-tomas");
    assert_eq!(body["context_id"], "c-garden");
    assert_eq!(body["confirmed_by"]["gesture"], "Hold");
    advance(cx, 300);
    let members = with_store(cx, &shell, |s| {
        s.context(&ContextId::new("c-garden")).map(|c| c.participants.len())
    });
    assert_eq!(members, Some(3));
    assert!(
        exists_in(cx, handle, "moment-m-removed", "governance-record"),
        "the governance moment"
    );

    let store = store_handle(cx, &shell);
    let path = cx.update(|cx| {
        store.read(cx).inspector().root_intents().find_map(|entry| match entry {
            ramflux_desktop_ui::app::InspectorEntry::UiEvent {
                path,
                kind: "GovernanceRequested",
                ..
            } => Some(path.clone()),
            _ => None,
        })
    });
    assert_eq!(path.as_deref(), Some("Workspace > Context(c-garden) > Panel(dock.members)"));
}

#[gpui_kit::test]
fn approvals_dock_revokes_with_a_hold_and_lists_the_audit(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s03_intervention");
    advance(cx, 1_000);
    click(cx, handle, "rail-approvals");
    assert!(exists(cx, handle, "dock-content-approvals"));

    in_window(cx, handle, |window, cx| {
        window.within("approvals-grant-g-calendar").click("revoke", cx)
    });
    assert!(strip_open(cx, &shell), "revoking needs the hold");
    hold_confirm(cx, handle);
    let body = last_body(cx, &shell, methods::GRANT_REVOKE);
    assert_eq!(body["grant_id"], "g-calendar");
    assert_eq!(body["confirmed_by"]["gesture"], "Hold");
    advance(cx, 200);
    let state = with_store(cx, &shell, |s| s.grant(&GrantId::new("g-calendar")).map(|g| g.state));
    assert_eq!(state, Some(GrantState::Revoked));

    in_window(cx, handle, |window, cx| {
        window.within("approvals-standing-sa-calendar").click("revoke", cx)
    });
    hold_confirm(cx, handle);
    assert_eq!(last_body(cx, &shell, methods::STANDING_REVOKE)["approval_id"], "sa-calendar");

    in_window(cx, handle, |window, cx| {
        window.within("approvals-pending-g-rotate").click("approve", cx)
    });
    assert!(
        strip_open(cx, &shell),
        "a high-risk pending grant decided from the dock still needs the hold"
    );
    let audit = with_store(cx, &shell, |s| s.audit().len());
    assert!(audit >= 3, "the audit lists the confirmed revocations");
}

#[gpui_kit::test]
fn evidence_dock_is_read_only(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s03_intervention");
    advance(cx, 1_000);
    click(cx, handle, "rail-evidence");
    assert!(exists(cx, handle, "dock-content-evidence"));
    let workspace = workspace(cx, &shell);
    let open = cx.update(|cx| workspace.read(cx).open_tool_kind());
    assert_eq!(open, Some(DockTool::Evidence));
    assert!(effects(cx, &shell).is_empty(), "showing evidence sends nothing");
    let franking = with_store(cx, &shell, |s| {
        s.moment(&MomentId::new("m-ask")).map(|m| format!("{:?}", m.kind).contains("frank:"))
    });
    assert_eq!(franking, Some(true));
}

#[gpui_kit::test]
fn summoned_docks_close_on_context_change_and_pinned_docks_stay(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    let workspace = workspace(cx, &shell);
    let open_tool = |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).open_tool_kind());
    click(cx, handle, "rail-objects");
    assert_eq!(open_tool(cx), Some(DockTool::Objects));
    show_context(cx, handle, &shell, "c-lena");
    assert_eq!(open_tool(cx), None, "a summoned tool closes when the context changes");

    click(cx, handle, "rail-objects");
    click(cx, handle, "dock-pin-objects");
    let pinned = cx.update(|cx| workspace.read(cx).pinned_tool());
    assert_eq!(pinned, Some(DockTool::Objects));
    show_context(cx, handle, &shell, "c-orbit");
    assert_eq!(open_tool(cx), Some(DockTool::Objects), "a pinned tool stays open");
    assert!(effects(cx, &shell).is_empty(), "dock layout is view-local");
}
