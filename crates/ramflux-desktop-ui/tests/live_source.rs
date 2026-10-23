// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The production window root on the SDK source over a fake daemon:
//! the entry page with the real daemon status, account sessions that follow
//! the daemon, controls the backend cannot perform shown disabled, and lock
//! returning to entry.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use ramflux_desktop_core::{AccountId, ContextId, Listing, SourceStatus};
use ramflux_desktop_sdk::SdkConfig;
use ramflux_desktop_test::fake_bus::{FakeBus, Reply, socket_path};
use ramflux_desktop_ui::route::{EntryStep, Page};
use ramflux_desktop_ui::shell::{ShellOptions, SourceChoice};
use ramflux_desktop_ui::workspace::PrefsStore;
use serde_json::{Value, json};

fn frames() -> BTreeMap<String, Value> {
    let text = include_str!("../../ramflux-desktop-sdk/tests/fixtures/rfd_frames.json");
    serde_json::from_str(text).expect("fixture json")
}

fn options(path: &std::path::Path) -> ShellOptions {
    let config = SdkConfig {
        request_timeout: Duration::from_secs(3),
        poll_fast: Duration::from_millis(80),
        poll_slow: Duration::from_millis(300),
        reconnect_backoff: Duration::from_millis(150),
        ..SdkConfig::new(path)
    };
    ShellOptions {
        source: SourceChoice::Rfd(config),
        scenario: String::new(),
        start_clock: true,
        wall_clock: false,
        prefs: PrefsStore::in_memory(),
    }
}

/// Lets the bridge thread answer while the GPUI test clock ticks.
fn wait(cx: &mut TestAppContext, what: &str, mut done: impl FnMut(&mut TestAppContext) -> bool) {
    for _ in 0..400 {
        if done(cx) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
        advance(cx, 50);
    }
    panic!("timed out waiting for: {what}");
}

#[gpui_kit::test]
fn an_empty_daemon_shows_the_node_step_with_its_status(cx: &mut TestAppContext) {
    let path = socket_path("ui-empty");
    let recorded = frames();
    let fake = FakeBus::start(&path, move |method, _, _| {
        Reply::Ok(recorded.get(&format!("{method}#recorded")).cloned().unwrap_or(json!({})))
    });
    let (handle, shell) = open_with(cx, options(&path));
    wait(cx, "connected", |cx| {
        with_store(cx, &shell, |s| s.source_status() == SourceStatus::Connected)
    });
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Node), "no node yet");
    assert!(exists(cx, handle, "entry-daemon-info"), "the daemon's socket and accounts show");
    wait(cx, "nodes listed", |cx| with_store(cx, &shell, |s| s.is_listed(Listing::Nodes)));
    assert!(exists(cx, handle, "entry-node-empty"), "an empty node list is an explicit state");
    assert!(!exists(cx, handle, "entry-node-loading"), "no perpetual spinner");
    let label = in_window(cx, handle, |window, _| {
        window.find("entry-node-empty").label().map(str::to_owned)
    });
    assert_eq!(label.as_deref(), Some(ramflux_desktop_ui::i18n::tr("entry.node.none").as_ref()));
    assert!(cx.update(|cx| shell.read(cx).store().read(cx).is_live()));
    fake.shutdown();
}

#[gpui_kit::test]
fn an_unlocked_account_opens_the_workspace_and_lock_returns_to_entry(cx: &mut TestAppContext) {
    let path = socket_path("ui-account");
    let locked = Arc::new(AtomicBool::new(false));
    let flag = locked.clone();
    let all = frames();
    let fake = FakeBus::start(&path, move |method, _, _| {
        if method == "account.lock" {
            flag.store(true, Ordering::SeqCst);
            return Reply::Ok(json!({ "local_account_id": "acct-main", "locked": true }));
        }
        if method == "account.list" && flag.load(Ordering::SeqCst) {
            return Reply::Ok(json!({ "active_account_id": null, "accounts": [] }));
        }
        Reply::Ok(all.get(method).cloned().unwrap_or(json!({})))
    });
    let (handle, shell) = open_with(cx, options(&path));
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    let active = cx.update(|cx| shell.read(cx).store().read(cx).active_account().cloned());
    assert_eq!(active, Some(AccountId::new("acct-main")), "the account has its own session");
    wait(cx, "the direct conversation", |cx| {
        with_store(cx, &shell, |s| s.context(&ContextId::new("dm-ana")).is_some())
    });
    show_context(cx, handle, &shell, "dm-ana");
    assert!(
        exists(cx, handle, "composer-unavailable"),
        "a direct send the daemon cannot route is shown disabled with its reason"
    );
    show_context(cx, handle, &shell, "g-team");
    assert!(!exists(cx, handle, "composer-unavailable"), "groups can be sent to");

    cx.update(|cx| {
        shell.update(cx, |shell, cx| {
            shell.open_settings_at(ramflux_desktop_ui::settings::SettingsSection::Account, cx)
        })
    });
    cx.run_until_parked();
    click(cx, handle, "settings-lock");
    wait(cx, "entry after lock", |cx| matches!(page(cx, &shell), Page::Entry(_)));
    assert!(locked.load(Ordering::SeqCst), "the daemon locked the account");
    let sessions = cx.update(|cx| shell.read(cx).store().read(cx).accounts().count());
    assert_eq!(sessions, 0, "the account's session and its cached data are gone");
    fake.shutdown();
}
