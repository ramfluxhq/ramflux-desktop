// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI integration tests of what the daemon of product TRAIN-01 added, in the
//! production window on the fake daemon (DESK-PIN-01): a group's roster and
//! signed name changing by sync (GRP-SYNC-01) as records in the group's
//! conversation, and an authenticated kind that does not fit the role of
//! what a member did (KIND-01-FU-02). Every assertion reads the text from
//! the accessibility snapshot, the same text the screen shows.

mod common;
mod sdk_fake;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use ramflux_desktop_core::{ContextId, GovernanceAction, MomentKind, ParticipantId, selectors};
use ramflux_desktop_ui::i18n::{self, tr_args};
use ramflux_desktop_ui::route::Page;
use sdk_fake::{GROUP, wait};
use serde_json::{Value, json};

const ACCOUNT: &str = "acct-main";

fn group_row(epoch: u64, name: &str, members: &[(&str, &str)]) -> Value {
    let routes: Vec<Value> = members
        .iter()
        .map(|(device, principal)| {
            json!({ "member_id": device, "principal_id": principal,
                    "member_principal_commitment": format!("cm-{principal}"),
                    "device_signing_public_key": format!("key-{principal}"),
                    "target_delivery_id": format!("target-{principal}"),
                    "federation": null, "delivery_target_known": true })
        })
        .collect();
    json!({ "group_id": "g-team", "id": "g-team", "group_epoch": epoch, "name": name,
            "name_epoch": epoch, "member_routes": routes,
            "conversation_id": GROUP, "conversation_ids": [GROUP] })
}

fn labels_of(cx: &mut TestAppContext, world: &World, id: &'static str) -> Vec<String> {
    in_window(cx, world.handle, move |window, _| {
        window.find_all(id).iter().filter_map(|e| e.label().map(str::to_owned)).collect()
    })
}

struct World {
    fake: ramflux_desktop_test::fake_bus::FakeBus,
    state: sdk_fake::Shared,
    handle: gpui_kit::WindowHandle<gpui_kit::base::Root>,
    shell: gpui_kit::Entity<ramflux_desktop_ui::Shell>,
}

fn open_group(cx: &mut TestAppContext, name: &str) -> World {
    let (fake, state, path) = sdk_fake::start(name);
    fake.enable_stream();
    state.lock().expect("state").groups = Some(vec![group_row(
        3,
        "Team",
        &[("device-me", "principal-me"), ("device-bo", "principal-bo")],
    )]);
    let (handle, shell) = open_sized(cx, sdk_fake::options(&path), 1_100.);
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "the group", |cx| {
        with_store(cx, &shell, |s| s.context(&ContextId::new(GROUP)).is_some())
    });
    show_context(cx, handle, &shell, GROUP);
    World { fake, state, handle, shell }
}

#[gpui_kit::test]
fn a_roster_and_name_change_by_sync_are_records_in_the_group(cx: &mut TestAppContext) {
    let world = open_group(cx, "ui-group-sync");
    advance(cx, 200);
    // The first listing is the baseline: no record of the roster it shows.
    assert!(!exists(cx, world.handle, "governance-record"));
    assert!(!exists(cx, world.handle, "group-rename"));

    // The creator renames the group and adds Ana, removes Bo: the daemon
    // adopts the signed state and announces it.
    world.state.lock().expect("state").groups = Some(vec![group_row(
        4,
        "Release review",
        &[("device-me", "principal-me"), ("device-ana", "principal-ana")],
    )]);
    world.fake.publish(ACCOUNT, "group.changed", json!({ "group_id": "g-team" }));
    let group = ContextId::new(GROUP);
    wait(cx, "the records", |cx| {
        with_store(cx, &world.shell, |s| {
            let timeline = selectors::timeline(s, &group);
            timeline.iter().any(|m| matches!(&m.kind, MomentKind::GroupRename(_)))
                && timeline.iter().filter(|m| matches!(&m.kind, MomentKind::Governance(_))).count()
                    == 2
        })
    });
    wait(cx, "the rename line", |cx| exists(cx, world.handle, "group-rename"));

    let rename = labels_of(cx, &world, "group-rename");
    let expected =
        tr_args("moment.group_rename.line", &[("old", "Team"), ("new", "Release review")]);
    assert_eq!(rename, [expected.to_string()], "the old and the new name, as displayed");

    // Ana joined and Bo was removed, each by the name the screen uses.
    let name_of = |cx: &mut TestAppContext, principal: &str| {
        with_store(cx, &world.shell, |s| {
            let id = ParticipantId::new(principal);
            s.participant(&id).map_or_else(|| id.short(), |p| p.display.clone())
        })
    };
    let (ana, bo) = (name_of(cx, "principal-ana"), name_of(cx, "principal-bo"));
    let record = |name: &str, action| {
        tr_args(
            "moment.governance.record",
            &[("name", name), ("action", &i18n::governance_action(action))],
        )
        .to_string()
    };
    let mut shown = labels_of(cx, &world, "governance-record");
    shown.sort();
    let mut wanted =
        vec![record(&ana, GovernanceAction::Joined), record(&bo, GovernanceAction::Removed)];
    wanted.sort();
    assert_eq!(shown, wanted);

    // DESK-PIN-03 L2: the signed state says who is gone, not who removed
    // them, so the removal row is announced without an author and shows no
    // header naming the removed person as if they had posted it.
    let row_id = format!("moment-group-sync:{GROUP}:Removed:principal-bo:4");
    let announced =
        in_window(cx, world.handle, |window, _| window.find(row_id).label().map(str::to_owned));
    let kind =
        i18n::moment_kind(&MomentKind::Governance(ramflux_desktop_core::GovernancePayload {
            action: GovernanceAction::Removed,
            target: ParticipantId::new("principal-bo"),
            role: None,
            unattributed: true,
        }));
    let at = labels_of(cx, &world, "governance-record");
    assert_eq!(at.len(), 2);
    let announced = announced.unwrap_or_default();
    assert!(announced.contains(kind.as_ref()), "{announced}");
    assert!(!announced.contains(&bo), "no author is announced: {announced}");

    // The same listing again (a replayed event) adds nothing.
    world.fake.publish(ACCOUNT, "group.changed", json!({ "group_id": "g-team" }));
    advance(cx, 300);
    assert_eq!(labels_of(cx, &world, "group-rename").len(), 1);
    assert_eq!(labels_of(cx, &world, "governance-record").len(), 2);
    world.fake.shutdown();
}

#[gpui_kit::test]
fn an_authenticated_kind_that_does_not_fit_the_action_is_named_as_such(cx: &mut TestAppContext) {
    let (fake, state, path, tasks) = sdk_fake::start_with_tasks("ui-kind-mismatch");
    tasks.announce_messages(true);
    tasks.report_event_kinds(true);
    // Ana's signed profile says she is an unattended AI; that does not fit
    // accepting a task, which is a human-class action.
    tasks.set_kind("principal-ana", "headless_ai");
    // The assignee's actor row on the card carries a kind the source did not
    // verify (it does not check the device for that row).
    tasks.set_kind(sdk_fake::BOT, "human");
    let (handle, shell) = open_with(cx, sdk_fake::options(&path));
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "the group", |cx| {
        with_store(cx, &shell, |s| s.context(&ContextId::new(GROUP)).is_some())
    });
    show_context(cx, handle, &shell, GROUP);
    tasks.propose(GROUP, "t-plan", "Draft the release notes", &[("a", "Draft")]).unwrap();
    tasks.accept_as("principal-ana", GROUP, "t-plan").unwrap();
    wait(cx, "the line", |cx| exists(cx, handle, "task-event-kind"));

    let kind = i18n::peer_kind(ramflux_desktop_core::PeerKind::HeadlessAi);
    let expected = tr_args("identity.kind.role_mismatch", &[("kind", &kind)]).to_string();
    let marks: Vec<String> = in_window(cx, handle, |window, _| {
        window
            .find_all("task-event-kind")
            .iter()
            .filter_map(|e| e.label().map(str::to_owned))
            .collect()
    });
    assert_eq!(marks, [expected], "the kind is named as not valid for the action");
    // The card's actor row says only that the kind is not verified for this
    // action, never that the role does not fit.
    let human = i18n::peer_kind(ramflux_desktop_core::PeerKind::Human);
    let neutral = tr_args("identity.kind.unverified_for_action", &[("kind", &human)]).to_string();
    wait(cx, "the card's actor row", |cx| exists(cx, handle, "task-kind"));
    let card: Vec<String> = in_window(cx, handle, |window, _| {
        window.find_all("task-kind").iter().filter_map(|e| e.label().map(str::to_owned)).collect()
    });
    assert_eq!(card, [neutral]);
    let claimed = tr_args("identity.kind.claimed", &[("kind", &kind)]).to_string();
    assert!(!marks.contains(&claimed), "never shown as a claim");
    let _ = state;
    fake.shutdown();
}

#[gpui_kit::test]
fn a_kind_the_source_verified_is_named_from_the_source_not_the_directory(cx: &mut TestAppContext) {
    // DESK-PIN-03 L5. Ana's device-matched profile kind arrives with the event
    // (`actor_kind`) and the source verified it for the action; the directory
    // holds no profile-signed claim of her. The line names the kind the
    // source reports instead of the unnamed "Kind verified".
    let (fake, _state, path, tasks) = sdk_fake::start_with_tasks("ui-kind-verified");
    tasks.announce_messages(true);
    tasks.report_event_kinds(true);
    tasks.set_kind("principal-ana", "human");
    tasks.verify_kind("principal-ana");
    let (handle, shell) = open_with(cx, sdk_fake::options(&path));
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "the group", |cx| {
        with_store(cx, &shell, |s| s.context(&ContextId::new(GROUP)).is_some())
    });
    show_context(cx, handle, &shell, GROUP);
    tasks.propose(GROUP, "t-plan", "Draft the release notes", &[("a", "Draft")]).unwrap();
    tasks.accept_as("principal-ana", GROUP, "t-plan").unwrap();
    wait(cx, "the line", |cx| exists(cx, handle, "task-event-kind"));
    let human = i18n::peer_kind(ramflux_desktop_core::PeerKind::Human);
    let named = tr_args("identity.kind.verified", &[("kind", &human)]).to_string();
    let unnamed = i18n::tr("identity.kind.verified_unnamed").to_string();
    let marks: Vec<String> = in_window(cx, handle, |window, _| {
        window
            .find_all("task-event-kind")
            .iter()
            .filter_map(|e| e.label().map(str::to_owned))
            .collect()
    });
    assert_eq!(marks, [named], "the verified kind is named; not {unnamed:?}");
    fake.shutdown();
}

#[gpui_kit::test]
fn the_chinese_unverified_kind_reads_as_not_verified_for_this_action(cx: &mut TestAppContext) {
    // DESK-PIN-03 L8.
    cx.update(|cx| {
        gpui_kit::init(cx);
        ramflux_desktop_ui::init(cx);
        i18n::select_locale("zh-CN", cx);
    });
    let kind = i18n::peer_kind(ramflux_desktop_core::PeerKind::Human);
    let text = tr_args("identity.kind.unverified_for_action", &[("kind", &kind)]).to_string();
    assert_eq!(text, format!("{kind}，未针对此操作验证"));
}
