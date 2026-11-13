// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The workspace sections beside the conversation list: the group
//! directory with its member panel, the contact profile, and the strangers
//! area where replying never accepts.
mod common;
use common::*;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{Entity, TestAppContext};
use ramflux_desktop_core::{
    ContactRequest, DomainEvent, ParticipantId, RequestDirection, RequestState,
};
use ramflux_desktop_test::load_fixture;
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::workspace::{DockTool, WorkspaceRoute};

type Handle = gpui_kit::WindowHandle<gpui_kit::base::Root>;

fn navigate(cx: &mut TestAppContext, handle: Handle, shell: &Entity<Shell>, route: WorkspaceRoute) {
    let workspace = workspace(cx, shell);
    in_window(cx, handle, |window, cx| {
        workspace.update(cx, |w, cx| w.navigate(route, window, cx));
    });
}

#[gpui_kit::test]
fn a_group_chosen_in_the_directory_opens_its_conversation_and_member_panel(
    cx: &mut TestAppContext,
) {
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s05_group_governance");
    advance(cx, 1000);
    navigate(cx, handle, &shell, WorkspaceRoute::Groups);
    assert!(exists(cx, handle, "group-directory"), "the groups route lists groups");
    assert!(
        exists(cx, handle, "groups-detail-empty"),
        "no group is shown until one is chosen (C05)"
    );
    let workspace = workspace(cx, &shell);
    let first = cx
        .update(|cx| workspace.read(cx).group_directory().read(cx).entry_ids(cx))
        .into_iter()
        .next()
        .expect("test: the scenario has a group");
    in_window(cx, handle, |window, cx| {
        workspace.update(cx, |w, cx| w.show_group(first.clone(), window, cx));
    });
    assert_eq!(
        cx.update(|cx| workspace.read(cx).open_tool_kind()),
        Some(DockTool::Members),
        "a chosen group shows its member panel beside the conversation (C02)"
    );
    assert!(exists(cx, handle, "group-profile"), "the member panel is headed by the profile");
}

#[gpui_kit::test]
fn a_contact_profile_compares_before_it_verifies(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 1000);
    navigate(cx, handle, &shell, WorkspaceRoute::Contacts);
    assert!(exists(cx, handle, "contact-directory"));
    assert!(exists(cx, handle, "contact-profile-empty"), "nothing is shown before a choice");
    let workspace = workspace(cx, &shell);
    let contact = cx
        .update(|cx| workspace.read(cx).contact_directory().read(cx).entry_ids(cx))
        .into_iter()
        .next();
    let Some(contact) = contact else { return };
    let profile = cx.update(|cx| workspace.read(cx).contact_profile().clone());
    cx.update(|cx| profile.update(cx, |p, cx| p.set_participant(Some(contact), cx)));
    cx.run_until_parked();
    assert!(exists(cx, handle, "contact-safety-number"), "the safety number is shown");
    let before = effects(cx, &shell).len();
    if exists(cx, handle, "contact-compare") {
        click(cx, handle, "contact-compare");
        assert!(
            exists(cx, handle, "contact-compare-cancel"),
            "Compare opens the comparison instead of verifying"
        );
        assert_eq!(effects(cx, &shell).len(), before, "opening a comparison sends nothing");
    }
}

#[gpui_kit::test]
fn replying_to_a_stranger_is_unavailable_and_never_accepts(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s02_daily");
    let mut scenario = load_fixture("s02_daily").expect("test: scenario");
    let request = ContactRequest {
        id: "req-stranger".into(),
        peer: ParticipantId::new("p-stranger"),
        direction: RequestDirection::Incoming,
        state: RequestState::Pending,
        note: Some("Hello from the review".into()),
        safety_number: Some("73051186442090573312".into()),
        held_messages: 2,
        preview: None,
        expires_at: None,
    };
    scenario.timeline.push((10, DomainEvent::ContactRequestUpserted(request)));
    scenario.timeline.sort_by_key(|(at, _)| *at);
    let store = store_handle(cx, &shell);
    cx.update(|cx| store.update(cx, |s, cx| s.load_scenario(&scenario, cx)));
    advance(cx, 1000);
    navigate(cx, handle, &shell, WorkspaceRoute::Strangers);
    assert!(exists(cx, handle, "strangers-area"));
    let workspace = workspace(cx, &shell);
    let area = cx.update(|cx| workspace.read(cx).strangers().clone());
    cx.update(|cx| area.update(cx, |a, cx| a.select(Some("req-stranger".into()), cx)));
    cx.run_until_parked();
    in_window(cx, handle, |w, _| {
        assert_eq!(
            w.find("stranger-reply").disabled(),
            Some(true),
            "reply without acceptance has no backend contract yet (typed unavailable)"
        );
        assert_eq!(w.find("stranger-report").disabled(), Some(true), "report is unavailable");
        assert_ne!(w.find("stranger-accept").disabled(), Some(true), "accept stays available");
    });
    let before = effects(cx, &shell).len();
    click(cx, handle, "stranger-reply");
    assert_eq!(effects(cx, &shell).len(), before, "a disabled reply sends nothing");
}
