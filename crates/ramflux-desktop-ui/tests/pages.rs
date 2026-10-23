// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI integration tests of the entry page (every path in, TOFU confirm) and
//! the settings page (each section's controls map to intents).

mod common;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use ramflux_desktop_core::{DeviceId, LockState, NodeId, methods};
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

/// s01 until the invite node is confirmed (no hold for an invite pin).
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
    assert!(body["confirmed_by"].is_null(), "an invitation pin needs no hold");
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
fn entry_adds_a_tofu_node_with_a_hold_then_creates_an_account(cx: &mut TestAppContext) {
    let (handle, shell) = harbor_confirmed(cx);
    click(cx, handle, "entry-review-nodes");
    type_into(cx, handle, "node-url", "https://studio.example.org");
    click(cx, handle, "node-add");
    assert_eq!(last_body(cx, &shell, methods::NODE_ADD)["url"], "https://studio.example.org");
    advance(cx, 400);
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Node));
    let confirms = |cx: &mut TestAppContext| {
        effects(cx, &shell).iter().filter(|l| *l == methods::NODE_CONFIRM).count()
    };
    let before = confirms(cx);
    click_in(cx, handle, "node-n-studio", "node-confirm");
    assert!(strip_open(cx, &shell), "trusting a key on first use needs the hold");
    assert_eq!(confirms(cx), before, "nothing is pinned before the hold");
    hold_confirm(cx, handle);
    let body = last_body(cx, &shell, methods::NODE_CONFIRM);
    assert_eq!(body["node_id"], "n-studio");
    assert_eq!(body["fingerprint"], "SHA256:e21f 9c07 3b5a 0d88 41e6 f2c9 7a13 5d06");
    assert_eq!(body["confirmed_by"]["gesture"], "Hold");
    advance(cx, 200);
    assert!(with_store(cx, &shell, |s| s
        .node(&NodeId::new("n-studio"))
        .is_some_and(|n| n.confirmed)));
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Account));
    click(cx, handle, "entry-continue");
    click(cx, handle, "entry-node-choice-n-studio");
    click(cx, handle, "entry-kind-1");
    type_into(cx, handle, "account-label", "Night shift");
    type_into(cx, handle, "account-secret", "river stone lantern");
    click(cx, handle, "account-create");
    let body = last_body(cx, &shell, methods::ACCOUNT_CREATE);
    assert_eq!(body["node_id"], "n-studio");
    assert_eq!(body["kind"], "HeadlessAi");
    assert_eq!(body["label"], "Night shift");
    assert!(!body.to_string().contains("river"), "the secret travels beside the body");
    assert!(secrets_cleared(cx, &shell), "the secret leaves the input on submit");
    assert!(!inspector_text(cx, &shell).contains("river"));
    advance(cx, 500);
    assert_eq!(page(cx, &shell), Page::Workspace);
}

#[gpui_kit::test]
fn entry_joins_as_a_new_device(cx: &mut TestAppContext) {
    let (handle, shell) = harbor_confirmed(cx);
    click(cx, handle, "entry-path-1");
    click(cx, handle, "entry-node-choice-n-harbor");
    type_into(cx, handle, "device-label", "Studio laptop");
    type_into(cx, handle, "activation-code", "4421-9087");
    click(cx, handle, "account-join");
    let body = last_body(cx, &shell, methods::DEVICE_ACTIVATE);
    assert_eq!(body["node_id"], "n-harbor");
    assert_eq!(body["device_label"], "Studio laptop");
    assert!(secrets_cleared(cx, &shell));
    advance(cx, 600);
    assert_eq!(page(cx, &shell), Page::Workspace);
    assert_eq!(with_store(cx, &shell, |s| s.devices().count()), 2);
}

#[gpui_kit::test]
fn entry_restores_a_backup_then_locks_and_unlocks(cx: &mut TestAppContext) {
    let (handle, shell) = harbor_confirmed(cx);
    click(cx, handle, "entry-path-2");
    type_into(cx, handle, "backup-path", "~/Documents/ramflux.backup");
    type_into(cx, handle, "backup-secret", "river stone lantern");
    click(cx, handle, "account-restore");
    assert_eq!(
        last_body(cx, &shell, methods::ACCOUNT_BACKUP_IMPORT)["path"],
        "~/Documents/ramflux.backup"
    );
    advance(cx, 700);
    assert_eq!(page(cx, &shell), Page::Workspace);

    settings(cx, handle, &shell, SettingsSection::Account);
    click(cx, handle, "settings-lock");
    assert_eq!(last_body(cx, &shell, methods::ACCOUNT_LOCK)["account_id"], "a-main");
    advance(cx, 100);
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Account), "locking returns to entry");
    assert!(!cx.update(|cx| shell.read(cx).is_settings_open()));

    type_into(cx, handle, "passphrase", "river stone lantern");
    in_window(cx, handle, |window, cx| window.press("enter", cx));
    assert_eq!(effects(cx, &shell).last().map(String::as_str), Some(methods::ACCOUNT_UNLOCK));
    assert!(secrets_cleared(cx, &shell));
    advance(cx, 200);
    let state = with_store(cx, &shell, |s| s.accounts().next().map(|a| a.lock_state));
    assert_eq!(state, Some(LockState::Unlocked));
    assert_eq!(page(cx, &shell), Page::Workspace);
}

#[gpui_kit::test]
fn settings_nodes_probe_set_default_and_guard_bound_nodes(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s09_multi_account_day");
    advance(cx, 1_000);
    settings(cx, handle, &shell, SettingsSection::Nodes);
    click_in(cx, handle, "settings-node-n-corp", "node-probe");
    assert_eq!(last_body(cx, &shell, methods::NODE_PROBE)["node_id"], "n-corp");
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
    click_in(cx, handle, "settings-node-n-harbor", "node-remove");
    assert_eq!(effects(cx, &shell).len(), before, "a bound node cannot be removed");
    click_in(cx, handle, "settings-node-n-ramflux", "node-enabled");
    assert_eq!(last_body(cx, &shell, methods::NODE_SET_ENABLED)["enabled"], false);
}

#[gpui_kit::test]
fn settings_devices_revoke_through_the_hold_and_sync(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s09_multi_account_day");
    advance(cx, 1_000);
    settings(cx, handle, &shell, SettingsSection::Devices);
    click_in(cx, handle, "device-dev-phone-01", "revoke");
    assert!(strip_open(cx, &shell), "revoking a device needs the hold");
    assert!(exists_in(cx, handle, "settings", "hold-confirm"), "the strip shows inside settings");
    hold_confirm(cx, handle);
    let body = last_body(cx, &shell, methods::DEVICE_REVOKE);
    assert_eq!(body["device_id"], "dev-phone-01");
    assert_eq!(body["confirmed_by"]["gesture"], "Hold");
    advance(cx, 300);
    assert!(with_store(cx, &shell, |s| s
        .device(&DeviceId::new("dev-phone-01"))
        .is_some_and(|d| d.revoked)));
    click(cx, handle, "sync-export");
    assert_eq!(effects(cx, &shell).last().map(String::as_str), Some(methods::DEVICE_SYNC_EXPORT));
}

#[gpui_kit::test]
fn settings_rotate_passphrase_and_export_diagnostics(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s09_multi_account_day");
    advance(cx, 1_000);
    settings(cx, handle, &shell, SettingsSection::Account);
    type_into(cx, handle, "rotate-current", "old words here");
    type_into(cx, handle, "rotate-next", "new words here");
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
    click(cx, handle, "diagnostics-export");
    assert_eq!(
        last_body(cx, &shell, methods::DIAGNOSTICS_EXPORT)["path"],
        "~/Documents/ramflux-diagnostics.txt"
    );
}
