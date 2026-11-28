// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI integration tests of the entry page (every path in, TOFU confirm) and
//! the settings page (each section's controls map to intents).

mod common;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use ramflux_desktop_core::{DeviceId, NodeId, methods};
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::route::{EntryStep, Page};
use ramflux_desktop_ui::settings::SettingsSection;

type Handle = gpui_kit::WindowHandle<gpui_kit::base::Root>;

fn secrets_cleared(cx: &mut TestAppContext, shell: &gpui_kit::Entity<Shell>) -> bool {
    let entry = cx.update(|cx| shell.read(cx).entry().clone());
    cx.update(|cx| entry.read(cx).secret_inputs().iter().all(|i| i.read(cx).value().is_empty()))
}

fn inspector_text(cx: &mut TestAppContext, shell: &gpui_kit::Entity<Shell>) -> String {
    cx.update(|cx| format!("{:?}", shell.read(cx).store().read(cx).inspector()))
}

/// s01 until the invite node is confirmed (no confirmation for an invite pin).
fn harbor_confirmed(cx: &mut TestAppContext) -> (Handle, gpui_kit::Entity<Shell>) {
    let (handle, shell) = open(cx, "s01_first_launch");
    advance(cx, 100);
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Daemon));
    advance(cx, 2_900);
    assert_eq!(
        page(cx, &shell),
        Page::Entry(EntryStep::Node),
        "the invite node waits for a decision"
    );
    click_in(cx, handle, "node-n-harbor", "node-confirm");
    let body = last_body(cx, &shell, methods::NODE_CONFIRM);
    assert_eq!(body["pin_source"], "Invite");
    assert!(body["confirmed_by"].is_null(), "an invitation pin needs no confirmation");
    advance(cx, 200);
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Account));
    (handle, shell)
}

fn settings(
    cx: &mut TestAppContext,
    handle: Handle,
    shell: &gpui_kit::Entity<Shell>,
    section: SettingsSection,
) {
    cx.update(|cx| shell.update(cx, |shell, cx| shell.open_settings_at(section, cx)));
    cx.run_until_parked();
    in_window(cx, handle, |_, _| {});
}

#[gpui_kit::test]
fn entry_adds_a_tofu_node_and_one_press_trusts_it(cx: &mut TestAppContext) {
    let (handle, shell) = harbor_confirmed(cx);
    click(cx, handle, "entry-review-nodes");
    type_into(cx, handle, "node-url", "https://studio.example.org");
    click(cx, handle, "node-add");
    assert_eq!(last_body(cx, &shell, methods::NODE_ADD)["url"], "https://studio.example.org");
    advance(cx, 400);
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Node));
    click_in(cx, handle, "node-n-studio", "node-confirm");
    assert!(!strip_open(cx, &shell), "the press of Trust this node is the confirmation");
    let body = last_body(cx, &shell, methods::NODE_CONFIRM);
    assert_eq!(body["node_id"], "n-studio");
    assert_eq!(body["fingerprint"], "SHA256:e21f 9c07 3b5a 0d88 41e6 f2c9 7a13 5d06");
    assert_eq!(body["confirmed_by"]["gesture"], "Tap");
    advance(cx, 200);
    assert!(with_store(cx, &shell, |s| s
        .node(&NodeId::new("n-studio"))
        .is_some_and(|n| n.confirmed)));
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Account));
}

#[gpui_kit::test]
fn entry_offers_only_the_full_restore(cx: &mut TestAppContext) {
    let (handle, _shell) = harbor_confirmed(cx);
    click(cx, handle, "entry-path-1");
    // Only the full restore is offered: the key backup import is gone.
    assert!(exists(cx, handle, "entry-full-restore"));
    assert!(!exists(cx, handle, "backup-browse"));
    assert!(!exists(cx, handle, "account-restore"));
}

#[gpui_kit::test]
fn settings_lock_shows_the_lock_page_and_unlock_sends_the_secret(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s09_multi_account_day");
    advance(cx, 1_000);
    assert_eq!(page(cx, &shell), Page::Workspace);
    settings(cx, handle, &shell, SettingsSection::Account);
    click(cx, handle, "settings-lock");
    assert!(last_body(cx, &shell, methods::ACCOUNT_LOCK)["account_id"].is_string());
    advance(cx, 100);
    assert_eq!(page(cx, &shell), Page::Lock, "locking shows the lock page, not the entry steps");
    assert!(!cx.update(|cx| shell.read(cx).is_settings_open()));

    type_into(cx, handle, "passphrase", "river stone lantern");
    in_window(cx, handle, |window, cx| window.press("enter", cx));
    assert_eq!(effects(cx, &shell).last().map(String::as_str), Some(methods::ACCOUNT_UNLOCK));
    assert!(secrets_cleared(cx, &shell));
}

#[gpui_kit::test]
fn settings_nodes_probe_set_default_and_guard_bound_nodes(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s09_multi_account_day");
    advance(cx, 1_000);
    settings(cx, handle, &shell, SettingsSection::Nodes);
    reveal(cx, handle, "settings-node-n-corp", "node-probe");
    click_in(cx, handle, "settings-node-n-corp", "node-probe");
    assert_eq!(last_body(cx, &shell, methods::NODE_PROBE)["node_id"], "n-corp");
    reveal(cx, handle, "settings-node-n-corp", "node-default");
    click_in(cx, handle, "settings-node-n-corp", "node-default");
    assert_eq!(last_body(cx, &shell, methods::NODE_SET_DEFAULT)["node_id"], "n-corp");
    advance(cx, 500);
    let (default, health) = with_store(cx, &shell, |s| {
        let corp = s.node(&NodeId::new("n-corp")).expect("node");
        (corp.is_default, corp.health)
    });
    assert!(default, "the local echo made it the default");
    assert_eq!(health, ramflux_desktop_core::NodeHealth::Healthy, "the probe result arrived");
    let before = effects(cx, &shell).len();
    reveal(cx, handle, "settings-node-n-harbor", "node-remove");
    click_in(cx, handle, "settings-node-n-harbor", "node-remove");
    assert_eq!(effects(cx, &shell).len(), before, "a bound node cannot be removed");
    reveal(cx, handle, "settings-node-n-ramflux", "node-enabled");
    click_in(cx, handle, "settings-node-n-ramflux", "node-enabled");
    assert_eq!(last_body(cx, &shell, methods::NODE_SET_ENABLED)["enabled"], false);
}

#[gpui_kit::test]
fn settings_devices_revoke_through_the_confirmation_and_sync(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s09_multi_account_day");
    advance(cx, 1_000);
    settings(cx, handle, &shell, SettingsSection::Devices);
    click_in(cx, handle, "device-dev-phone-01", "revoke");
    assert!(strip_open(cx, &shell), "revoking a device needs the confirmation");
    assert!(exists_in(cx, handle, "settings", "confirm-accept"), "the strip shows inside settings");
    tap_confirm(cx, handle);
    let body = last_body(cx, &shell, methods::DEVICE_REVOKE);
    assert_eq!(body["device_id"], "dev-phone-01");
    assert_eq!(body["confirmed_by"]["gesture"], "Tap");
    advance(cx, 300);
    assert!(with_store(cx, &shell, |s| s
        .device(&DeviceId::new("dev-phone-01"))
        .is_some_and(|d| d.revoked)));
    let before = effects(cx, &shell).len();
    click(cx, handle, "sync-export");
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    assert_eq!(effects(cx, &shell).len(), before, "a cancelled dialog exports nothing");
    click(cx, handle, "sync-export");
    cx.simulate_new_path_selection(|dir| Some(dir.join("sync.bundle")));
    cx.run_until_parked();
    assert_eq!(effects(cx, &shell).last().map(String::as_str), Some(methods::DEVICE_SYNC_EXPORT));
    assert!(
        last_body(cx, &shell, methods::DEVICE_SYNC_EXPORT)["path"]
            .as_str()
            .is_some_and(|p| p.ends_with("/sync.bundle"))
    );
}

#[gpui_kit::test]
fn settings_rotate_passphrase_and_export_diagnostics(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s09_multi_account_day");
    advance(cx, 1_000);
    settings(cx, handle, &shell, SettingsSection::Account);
    type_into(cx, handle, "rotate-current", "old words here");
    type_into(cx, handle, "rotate-next", "new words here");
    type_into(cx, handle, "rotate-repeat", "new words here");
    click(cx, handle, "settings-rotate");
    let body = last_body(cx, &shell, methods::ACCOUNT_ROTATE_PASSPHRASE);
    assert_eq!(body["account_id"], "a-personal");
    assert!(!body.to_string().contains("words"));
    let page = cx.update(|cx| shell.read(cx).settings().clone());
    let cleared =
        cx.update(|cx| page.read(cx).rotate_inputs().iter().all(|i| i.read(cx).value().is_empty()));
    assert!(cleared, "both passphrases leave their inputs");
    assert!(!inspector_text(cx, &shell).contains("words"));

    settings(cx, handle, &shell, SettingsSection::Diagnostics);
    assert!(exists(cx, handle, "diagnostics-preview"));
    assert!(
        !exists(cx, handle, "diagnostics-sync"),
        "scenarios have no daemon to keep them up to date (shown on a daemon: sync_ui)"
    );
    click(cx, handle, "diagnostics-export");
    cx.simulate_new_path_selection(|dir| Some(dir.join("diag.txt")));
    cx.run_until_parked();
    assert!(
        last_body(cx, &shell, methods::DIAGNOSTICS_EXPORT)["path"]
            .as_str()
            .is_some_and(|p| p.ends_with("/diag.txt"))
    );
}

/// A long nodes page scrolls to its last node: with many backup nodes every
/// node's actions can be brought on screen and used (the page's scroll range
/// covers all of its content, whatever its length).
#[gpui_kit::test]
fn settings_nodes_long_page_reaches_its_last_node(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s09_multi_account_day");
    advance(cx, 1_000);
    let store = store_handle(cx, &shell);
    cx.update(|cx| {
        store.update(cx, |h, _| {
            let mock = h.mock_mut().expect("a mock source");
            for n in 0..8 {
                let node: ramflux_desktop_core::Node = serde_json::from_value(serde_json::json!({
                    "id": format!("n-extra-{n}"),
                    "endpoints": [format!("quic://extra-{n}.example.net:7443")],
                    "fingerprint": "SHA256:0c55 a1e9 37f2 8b06 d4c1 9e7a 52b3 f810",
                    "pin_source": "Manual",
                    "capabilities": ["gateway", "relay"],
                }))
                .expect("node");
                mock.inject(0, ramflux_desktop_core::DomainEvent::NodeUpserted(node));
            }
        })
    });
    advance(cx, 100);
    settings(cx, handle, &shell, SettingsSection::Nodes);
    for id in ["settings-node-n-extra-0", "settings-node-n-extra-7"] {
        reveal(cx, handle, id, "node-enabled");
    }
    click_in(cx, handle, "settings-node-n-extra-7", "node-enabled");
    assert_eq!(last_body(cx, &shell, methods::NODE_SET_ENABLED)["enabled"], false);
}

/// Scrolls the nodes page until `child` of the node row `id` is on screen
/// (the page is the board's long column of rows, G09).
fn reveal(
    cx: &mut TestAppContext,
    handle: gpui_kit::WindowHandle<gpui_kit::base::Root>,
    id: &'static str,
    child: &'static str,
) {
    use gpui_kit::InputEvent as _;
    // The wheel goes where a person scrolls: the middle of the page area
    // between the page header and the window's bottom. (Aiming at a probe
    // button that is only partly on screen put the wheel under the page
    // header, which does not scroll, whenever such a button sat across the
    // header's edge.)
    let scroll = |cx: &mut TestAppContext, dy: f32| {
        in_window(cx, handle, |window, cx| {
            let Some(header) = window.try_find("settings-header").map(|h| h.bounds()) else {
                return;
            };
            let bottom = window.viewport_size().height;
            let position = gpui_kit::point(header.center().x, (header.bottom() + bottom) / 2.);
            let delta =
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(dy)));
            window.dispatch_event(
                gpui_kit::ScrollWheelEvent { position, delta, ..Default::default() }
                    .to_platform_input(),
                cx,
            );
        });
    };
    scroll(cx, 4_000.);
    for _ in 0..60 {
        // Wholly inside the page area (below the page header, above the
        // window's bottom), so a click on its centre reaches it.
        let shown = in_window(cx, handle, |window, _| {
            let Some(header) = window.try_find("settings-header").map(|h| h.bounds()) else {
                return false;
            };
            let bottom = window.viewport_size().height;
            window.within(id).try_find(child).is_some_and(|b| {
                let b = b.bounds();
                b.top() >= header.bottom() && b.bottom() <= bottom
            })
        });
        if shown {
            return;
        }
        scroll(cx, -150.);
    }
    panic!("{child} of {id} never came into view");
}
