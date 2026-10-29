// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI integration tests of what DESK-PIN-03 adds, in the production window on
//! the fake daemon. Every assertion reads the text from the accessibility
//! snapshot, the same text the screen shows.
//!
//! - The daemon's bound on remembered requester identities
//!   (`authority_floors`, product CONTACT-FU-03): the notice that new
//!   requests are on hold, and "Forget old entries" behind the hold, which
//!   sends the current setting with `clear_retired_authority_floors`.
//! - A group change refused `transfer_requires_sync` while the group's head
//!   is held for an ownership transfer: said in words, with the reason, and
//!   nothing offered to retry on the user's behalf.

mod common;
mod sdk_fake;

use std::sync::Arc;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use ramflux_desktop_core::{ContextId, SyncMode, methods};
use ramflux_desktop_test::fake_contact_requests::FakeContactRequests;
use ramflux_desktop_ui::i18n::{self, tr, tr_args};
use ramflux_desktop_ui::route::Page;
use sdk_fake::wait;
use serde_json::{Value, json};

struct World {
    fake: Arc<ramflux_desktop_test::fake_bus::FakeBus>,
    state: sdk_fake::Shared,
    requests: Arc<FakeContactRequests>,
    handle: gpui_kit::WindowHandle<gpui_kit::base::Root>,
    shell: gpui_kit::Entity<ramflux_desktop_ui::Shell>,
}

fn open_world(cx: &mut TestAppContext, name: &str, locale: &str) -> World {
    let (fake, state, path) = sdk_fake::start(name);
    fake.enable_stream();
    let fake = Arc::new(fake);
    let requests = state.lock().expect("state").requests.clone();
    let bus = Arc::downgrade(&fake);
    requests.set_publisher(Box::new(move |account, topic, body| {
        if let Some(bus) = bus.upgrade() {
            bus.publish(account, topic, body);
        }
    }));
    cx.update(|cx| {
        gpui_kit::init(cx);
        ramflux_desktop_ui::init(cx);
        i18n::select_locale(locale, cx);
    });
    state.lock().expect("state").groups = Some(vec![group_row()]);
    let (handle, shell) = open_sized(cx, sdk_fake::options(&path), 1_100.);
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "the stream", |cx| {
        with_store(cx, &shell, |s| s.sync_status().is_some_and(|x| x.mode == SyncMode::Stream))
    });
    World { fake, state, requests, handle, shell }
}

fn group_row() -> Value {
    let routes: Vec<Value> = [("device-me", "principal-me"), ("device-bo", "principal-bo")]
        .iter()
        .map(|(device, principal)| {
            json!({ "member_id": device, "principal_id": principal,
                    "member_principal_commitment": format!("cm-{principal}"),
                    "device_signing_public_key": format!("key-{principal}"),
                    "target_delivery_id": format!("target-{principal}"),
                    "federation": null, "delivery_target_known": true })
        })
        .collect();
    json!({ "group_id": "g-team", "id": "g-team", "group_epoch": 3, "name": "Team",
            "name_epoch": 3, "member_routes": routes,
            "conversation_id": sdk_fake::GROUP, "conversation_ids": [sdk_fake::GROUP] })
}

/// Clicks the menu item labeled `label` (popup menus are labeled rows).
fn click_label(
    cx: &mut TestAppContext,
    handle: gpui_kit::WindowHandle<gpui_kit::base::Root>,
    label: &str,
) {
    let label = label.to_owned();
    in_window(cx, handle, move |window, cx| {
        let at = window.find_by_label(&label).bounds().center();
        gpui_kit::test::TestEventExt::pointer_down(window, at, gpui_kit::MouseButton::Left, cx);
        gpui_kit::test::TestEventExt::pointer_up(window, at, gpui_kit::MouseButton::Left, cx);
    });
}

fn label(cx: &mut TestAppContext, world: &World, id: &'static str) -> Option<String> {
    in_window(cx, world.handle, move |window, _| {
        window.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
}

fn floors_text(retained: u32, clearable: u32, locale: &str) -> String {
    let (retained, clearable) = (retained.to_string(), clearable.to_string());
    let key = if clearable == "0" {
        "dock.contacts.floors.full"
    } else {
        "dock.contacts.floors.full_clearable"
    };
    let _ = locale;
    tr_args(key, &[("retained", &retained), ("limit", "4096"), ("clearable", &clearable)])
        .to_string()
}

fn open_contacts(cx: &mut TestAppContext, world: &World) {
    click(cx, world.handle, "rail-contacts");
    assert!(exists(cx, world.handle, "dock-content-contacts"));
}

fn sent(world: &World, method: &str) -> Vec<Value> {
    world.fake.seen().into_iter().filter(|s| s.method == method).map(|s| s.body).collect()
}

fn check_floors(cx: &mut TestAppContext, name: &str, locale: &str) {
    let world = open_world(cx, name, locale);
    // A daemon below its bound shows nothing; one that does not report it
    // shows nothing either.
    world.requests.set_floors(Some((10, 2, 4096)));
    open_contacts(cx, &world);
    advance(cx, 400);
    assert!(!exists(cx, world.handle, "contacts-floors"), "not saturated: no notice");

    // At the bound, with entries the user may forget.
    world.requests.set_floors(Some((4096, 3, 4096)));
    world.fake.publish("acct-main", "contact.requested", json!({}));
    wait(cx, "the notice", |cx| exists(cx, world.handle, "contacts-floors"));
    let text = label(cx, &world, "contacts-floors-text").unwrap_or_default();
    assert_eq!(text, floors_text(4096, 3, locale), "{locale}");
    let role = in_window(cx, world.handle, |window, _| window.find("contacts-floors-text").role());
    assert_eq!(role, Some(gpui_kit::Role::Status), "announced politely, not as an alert");
    let clear = tr_args("dock.contacts.floors.clear", &[("clearable", "3")]).to_string();
    assert_eq!(label(cx, &world, "contacts-floors-clear").as_deref(), Some(clear.as_str()));
    assert_ne!(clear, "dock.contacts.floors.clear", "a translation exists in {locale}");

    // The notice does not take the focus; pressing the button asks first,
    // nothing is sent, and the hold strip then takes the focus (a single
    // key press never fires it: keyboard.rs).
    let focus = |cx: &mut TestAppContext, id: &'static str| {
        in_window(cx, world.handle, |window, _| window.try_find(id).and_then(|e| e.focused()))
    };
    assert_ne!(focus(cx, "contacts-floors-clear"), Some(true), "the button is not focused");
    click(cx, world.handle, "contacts-floors-clear");
    assert_eq!(focus(cx, "hold-confirm"), Some(true), "the hold strip takes the focus");
    assert!(strip_open(cx, &world.shell), "forgetting needs the hold");
    assert!(sent(&world, methods::CONTACT_REQUESTS_POLICY).is_empty());
    assert!(sent(&world, "contact.requests.policy.set").is_empty());
    let asked = tr_args("confirm.contact.clear_floors", &[("count", "3")]).to_string();
    assert_ne!(asked, "confirm.contact.clear_floors");
    let announced = label(cx, &world, "confirm-strip").unwrap_or_default();
    assert!(announced.contains(&asked), "the hold says what it does: {announced}");
    // Escape declines it: nothing is sent, and the notice stays.
    in_window(cx, world.handle, |window, cx| window.press("escape", cx));
    assert!(!strip_open(cx, &world.shell), "Escape closes the hold");
    assert!(sent(&world, "contact.requests.policy.set").is_empty());
    assert!(exists(cx, world.handle, "contacts-floors"));
    click(cx, world.handle, "contacts-floors-clear");
    assert!(strip_open(cx, &world.shell));
    hold_confirm(cx, world.handle);
    wait(cx, "the daemon is asked", |_| !sent(&world, "contact.requests.policy.set").is_empty());
    assert_eq!(
        sent(&world, "contact.requests.policy.set"),
        [json!({ "contact_requests": "manual", "clear_retired_authority_floors": true })]
    );
    // The daemon forgot them: it takes requests again, the notice goes.
    wait(cx, "the notice is gone", |cx| !exists(cx, world.handle, "contacts-floors"));
    assert_eq!(world.requests.floors(), Some((4093, 0, 4096)));

    // At the bound with nothing to forget yet: the state is said, with no
    // action that would do nothing.
    world.requests.set_floors(Some((4096, 0, 4096)));
    world.fake.publish("acct-main", "contact.requested", json!({}));
    wait(cx, "the notice", |cx| exists(cx, world.handle, "contacts-floors"));
    assert_eq!(
        label(cx, &world, "contacts-floors-text").as_deref(),
        Some(floors_text(4096, 0, locale).as_str())
    );
    assert!(!exists(cx, world.handle, "contacts-floors-clear"));
}

/// The locale is process-wide: the tests of this binary take turns, each
/// running both languages and ending in English.
#[gpui_kit::test]
fn a_full_list_of_remembered_requesters_is_said_and_can_be_cleared(cx: &mut TestAppContext) {
    let _guard = locale_guard();
    check_floors(cx, "ui-floors-en", "en");
    check_floors(cx, "ui-floors-zh", "zh-CN");
    cx.update(|cx| i18n::select_locale("en", cx));
}

fn check_group_refusal(cx: &mut TestAppContext, name: &str, locale: &str) {
    let world = open_world(cx, name, locale);
    wait(cx, "the group", |cx| {
        with_store(cx, &world.shell, |s| s.context(&ContextId::new(sdk_fake::GROUP)).is_some())
    });
    show_context(cx, world.handle, &world.shell, sdk_fake::GROUP);
    click(cx, world.handle, "rail-members");
    assert!(exists(cx, world.handle, "dock-content-members"));
    let expected = [
        ("head_check_pending", "backend.group.sync_pending"),
        ("head_or_removal_floor_mismatch", "backend.group.handover_stale"),
        ("directory_unavailable", "backend.group.not_ready"),
        ("transfer_outcome_pending", "backend.group.transfer_pending"),
        ("clock_skew_or_expired_control", "backend.group.clock_skew"),
    ];
    for (reason, key) in expected {
        world.state.lock().expect("state").group_refusal = Some(reason);
        let before = with_store(cx, &world.shell, |s| s.failures().len());
        in_window(cx, world.handle, |window, cx| {
            window.within("member-principal-bo").click("member-actions", cx);
        });
        click_label(cx, world.handle, &tr("dock.members.kick"));
        assert!(strip_open(cx, &world.shell), "kicking needs the hold");
        hold_confirm(cx, world.handle);
        wait(cx, "the refusal", |cx| with_store(cx, &world.shell, |s| s.failures().len() > before));
        let shown = label(cx, &world, "status-failure-value").unwrap_or_default();
        let words = tr(key).to_string();
        assert_ne!(words, key, "a translation exists in {locale}");
        // The reason names the group, so the action word is not repeated.
        assert_eq!(shown, words, "{reason} in {locale}");
        assert!(!shown.contains("transfer_requires_sync"), "{shown}");
        // Nothing says the change was made or offers to retry it.
        assert!(!exists(cx, world.handle, "status-failure-retry"));
    }
    // The daemon was asked once for each of them.
    assert_eq!(sent(&world, "group.member.kick").len(), expected.len());
}

#[gpui_kit::test]
fn a_group_change_held_for_an_ownership_transfer_is_explained(cx: &mut TestAppContext) {
    let _guard = locale_guard();
    check_group_refusal(cx, "ui-transfer-en", "en");
    check_group_refusal(cx, "ui-transfer-zh", "zh-CN");
    cx.update(|cx| i18n::select_locale("en", cx));
}

#[gpui_kit::test]
fn the_chinese_unverified_kind_reads_as_not_verified_for_this_action(cx: &mut TestAppContext) {
    // DESK-PIN-03 L8.
    let _guard = locale_guard();
    cx.update(|cx| {
        gpui_kit::init(cx);
        ramflux_desktop_ui::init(cx);
        i18n::select_locale("zh-CN", cx);
    });
    let kind = i18n::peer_kind(ramflux_desktop_core::PeerKind::Human);
    let text = tr_args("identity.kind.unverified_for_action", &[("kind", &kind)]).to_string();
    // The kind, a full-width comma and "not verified for this action"
    // (Rust sources carry no CJK text).
    let expected =
        format!("{kind}\u{ff0c}\u{672a}\u{9488}\u{5bf9}\u{6b64}\u{64cd}\u{4f5c}\u{9a8c}\u{8bc1}");
    assert_eq!(text, expected);
    cx.update(|cx| i18n::select_locale("en", cx));
}
