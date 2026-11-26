// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The production window root, headless, on a real `rf daemon` from the
//! pinned product revision (source = rfd), without a node: the entry page
//! shows the daemon's status and the explicit empty node list, a stopped
//! daemon sends it back to the daemon step, and a restarted one is picked
//! up again; a daemon started from the app's own button is stopped
//! gracefully (`daemon.stop`, no signal).
//!
//! Ignored by default. Run with a pinned `rf`:
//!
//! ```sh
//! RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=/path/to/bin/rf \
//!   cargo test -p ramflux-desktop-ui --test rfd_live -- --ignored --test-threads=1
//! ```

mod common;

use std::time::Duration;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use ramflux_desktop_core::{Listing, SourceStatus};
use ramflux_desktop_sdk::SdkConfig;
use ramflux_desktop_test::live_daemon::{self, LiveDaemon};
use ramflux_desktop_ui::i18n::tr;
use ramflux_desktop_ui::route::{EntryStep, Page};
use ramflux_desktop_ui::shell::{ShellOptions, SourceChoice};
use ramflux_desktop_ui::workspace::PrefsStore;

fn options(daemon: &LiveDaemon) -> ShellOptions {
    options_at(&daemon.socket())
}

fn options_at(socket: &std::path::Path) -> ShellOptions {
    let config =
        SdkConfig { reconnect_backoff: Duration::from_millis(200), ..SdkConfig::new(socket) };
    ShellOptions {
        source: SourceChoice::Rfd(config),
        scenario: String::new(),
        start_clock: true,
        wall_clock: false,
        prefs: PrefsStore::in_memory(),
        launch: ramflux_desktop_ui::launch::LaunchOptions::in_memory(),
    }
}

/// Lets the bridge thread talk to the daemon while the GPUI clock ticks.
fn wait(cx: &mut TestAppContext, what: &str, mut done: impl FnMut(&mut TestAppContext) -> bool) {
    for _ in 0..600 {
        if done(cx) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
        advance(cx, 50);
    }
    panic!("timed out waiting for: {what}");
}

#[gpui_kit::test]
#[ignore = "needs a real rf daemon: RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=..."]
fn the_window_on_a_real_daemon_shows_the_empty_node_list_and_follows_a_restart(
    cx: &mut TestAppContext,
) {
    if !live_daemon::enabled() {
        println!("skipped: set {}=1 to run against a real daemon", live_daemon::ENABLE_ENV);
        return;
    }
    let mut daemon = LiveDaemon::start("ui").unwrap_or_else(|e| panic!("daemon: {e}"));
    let (handle, shell) = open_with(cx, options(&daemon));
    wait(cx, "nodes listed", |cx| {
        with_store(cx, &shell, |s| {
            s.source_status() == SourceStatus::Connected && s.is_listed(Listing::Nodes)
        })
    });
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Node));
    assert!(exists(cx, handle, "entry-daemon-info"), "the daemon's socket and accounts show");
    let empty = in_window(cx, handle, |window, _| {
        window.try_find("entry-node-empty").and_then(|e| e.label().map(str::to_owned))
    });
    assert_eq!(empty.as_deref(), Some(tr("entry.node.none").as_ref()), "explicit empty state");
    assert!(!exists(cx, handle, "entry-node-loading"), "no perpetual spinner");

    daemon.stop();
    wait(cx, "disconnected", |cx| {
        with_store(cx, &shell, |s| s.source_status() == SourceStatus::Disconnected)
    });
    daemon.restart().unwrap_or_else(|e| panic!("restart: {e}"));
    wait(cx, "reconnected", |cx| {
        with_store(cx, &shell, |s| s.source_status() == SourceStatus::Connected)
    });
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Node), "back on the node step");
    assert!(exists(cx, handle, "entry-node-empty"));
}

/// A short temporary directory for a daemon the app starts; removed on drop.
struct AppDaemonDir(std::path::PathBuf);

impl Drop for AppDaemonDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[gpui_kit::test]
#[ignore = "needs a real rf daemon: RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=..."]
fn a_daemon_started_from_the_app_is_stopped_gracefully(cx: &mut TestAppContext) {
    use ramflux_desktop_sdk::local::{LaunchSettings, LaunchStore, OwnedDaemonFile};
    use ramflux_desktop_ui::launch::{LaunchOptions, LaunchState};
    if !live_daemon::enabled() {
        println!("skipped: set {}=1 to run against a real daemon", live_daemon::ENABLE_ENV);
        return;
    }
    let rf = live_daemon::rf_binary().unwrap_or_else(|e| panic!("rf: {e}"));
    let dir = AppDaemonDir(std::path::PathBuf::from(format!("/tmp/rf-f7-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).expect("dir");
    let store = LaunchStore::at(dir.0.join("desktop-settings.json"));
    let settings = LaunchSettings {
        daemon_binary: Some(rf),
        data_dir: Some(dir.0.join("data")),
        ..LaunchSettings::default()
    };
    store.save(&settings).expect("settings");
    let owned = OwnedDaemonFile::at(dir.0.join("owned-daemon.json"));
    let socket = dir.0.join("rfd.sock");
    let mut options = options_at(&socket);
    options.launch = LaunchOptions {
        store,
        bundled: None,
        log_dir: Some(dir.0.join("logs")),
        owned: owned.clone(),
        ..LaunchOptions::default()
    };
    let (handle, shell) = open_with(cx, options);
    let launcher = cx.update(|cx| shell.read(cx).launcher().clone());
    click(cx, handle, "entry-daemon-start");
    wait(cx, "running", |cx| {
        cx.update(|cx| matches!(launcher.read(cx).state(), LaunchState::Running { .. }))
    });
    assert!(owned.load().is_some(), "the started daemon is recorded");
    cx.update(|cx| launcher.update(cx, |launcher, cx| launcher.stop(cx)));
    wait(cx, "stopped", |cx| cx.update(|cx| *launcher.read(cx).state() == LaunchState::Idle));
    assert_eq!(
        cx.update(|cx| launcher.read(cx).last_stop_graceful()),
        Some(true),
        "ended by daemon.stop, not by a signal"
    );
    assert!(!socket.exists(), "the daemon removed its socket");
    assert_eq!(owned.load(), None, "the record is cleared");
    wait(cx, "disconnected", |cx| {
        with_store(cx, &shell, |s| s.source_status() == SourceStatus::Disconnected)
    });
}

/// A daemon on a copy of the prepared data root (an account the daemon opens
/// by itself); `None` when live tests or the template are not switched on.
fn prepared_daemon(tag: &str) -> Option<LiveDaemon> {
    if !live_daemon::enabled() {
        println!("skipped: set {}=1 to run against a real daemon", live_daemon::ENABLE_ENV);
        return None;
    }
    let Some(template) = std::env::var_os(live_daemon::TEMPLATE_ENV) else {
        println!("skipped: set {} to a prepared data root", live_daemon::TEMPLATE_ENV);
        return None;
    };
    let daemon = LiveDaemon::start_from(tag, std::path::Path::new(&template))
        .unwrap_or_else(|e| panic!("daemon: {e}"));
    // The daemon opens the account by itself a moment after it answers; the
    // window lists accounts once when it connects.
    for _ in 0..200 {
        let listed = daemon.call(None, "account", "account.list", &serde_json::json!({}));
        if listed.is_ok_and(|l| l["account_details"][0]["state"] == "unlocked") {
            return Some(daemon);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("the daemon did not open the prepared account");
}

/// The id of the one account the prepared daemon has open.
fn open_account(daemon: &LiveDaemon) -> ramflux_desktop_core::AccountId {
    let listed = daemon
        .call(None, "account", "account.list", &serde_json::json!({}))
        .unwrap_or_else(|e| panic!("{e}"));
    let id = listed["accounts"][0]["id"].as_str().expect("an open account").to_owned();
    ramflux_desktop_core::AccountId::new(id)
}

/// Submits `kind` as the window's human would.
fn submit(
    cx: &mut TestAppContext,
    handle: gpui_kit::WindowHandle<gpui_kit::base::Root>,
    shell: &gpui_kit::Entity<ramflux_desktop_ui::shell::Shell>,
    kind: ramflux_desktop_core::IntentKind,
) -> String {
    let root = cx.update(|cx| shell.read(cx).root().clone());
    in_window(cx, handle, move |window, cx| {
        root.update(cx, |root, cx| {
            root.submit(ramflux_desktop_core::Intent::human(kind), window, cx)
        })
    })
}

/// The app lock the daemon session's store holds for `account`.
fn lock_state(
    cx: &mut TestAppContext,
    shell: &gpui_kit::Entity<ramflux_desktop_ui::shell::Shell>,
    account: &ramflux_desktop_core::AccountId,
) -> Option<ramflux_desktop_core::AppLock> {
    cx.update(|cx| shell.read(cx).store().read(cx).entry_store().app_lock(account).cloned())
}

fn configure_app_lock(
    cx: &mut TestAppContext,
    handle: gpui_kit::WindowHandle<gpui_kit::base::Root>,
    shell: &gpui_kit::Entity<ramflux_desktop_ui::shell::Shell>,
    account: &ramflux_desktop_core::AccountId,
    timeout_ms: u64,
) {
    let result = submit(
        cx,
        handle,
        shell,
        ramflux_desktop_core::IntentKind::ConfigureAppLock {
            account_id: account.clone(),
            enabled: true,
            timeout_ms,
        },
    );
    assert!(result.starts_with("submitted"), "{result}");
    wait(cx, "the lock is on", |cx| {
        lock_state(cx, shell, account)
            .is_some_and(|lock| lock.enabled && lock.timeout_ms == timeout_ms)
    });
}

/// The window goes to the background (the report the root sends) and then
/// nothing changes: no focus change, no input. The daemon's own timer locks
/// the account, and the window must show the lock page.
#[gpui_kit::test]
#[ignore = "needs a real rf daemon and a prepared account: RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=... RAMFLUX_LIVE_APP_LOCK_TEMPLATE=..."]
fn the_daemons_timeout_locks_an_unfocused_window_without_a_focus_change(cx: &mut TestAppContext) {
    let Some(daemon) = prepared_daemon("lock-timeout") else { return };
    let account = open_account(&daemon);
    let (handle, shell) = open_with(cx, options(&daemon));
    wait(cx, "the workspace", |cx| page(cx, &shell) == Page::Workspace);
    configure_app_lock(cx, handle, &shell, &account, 1_500);
    assert_eq!(page(cx, &shell), Page::Workspace);
    let result = submit(
        cx,
        handle,
        &shell,
        ramflux_desktop_core::IntentKind::AppLockLifecycle {
            account_id: account.clone(),
            phase: ramflux_desktop_core::AppLockPhase::Background,
        },
    );
    assert!(result.starts_with("submitted"), "{result}");
    let started = std::time::Instant::now();
    wait(cx, "the lock page", |cx| page(cx, &shell) == Page::Lock);
    println!("the lock page showed {:?} after the background report", started.elapsed());
    assert!(
        started.elapsed() >= Duration::from_millis(1_000),
        "the page cannot show before the daemon's deadline"
    );
    assert!(
        lock_state(cx, &shell, &account)
            .is_some_and(|lock| lock.reason.as_deref() == Some("timeout"))
    );
}

/// A second control client (as `rf account app-lock lock` is) locks the
/// account while the window stays in the foreground and untouched.
#[gpui_kit::test]
#[ignore = "needs a real rf daemon and a prepared account: RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=... RAMFLUX_LIVE_APP_LOCK_TEMPLATE=..."]
fn a_lock_sent_by_another_control_client_shows_the_lock_page(cx: &mut TestAppContext) {
    let Some(daemon) = prepared_daemon("lock-cli") else { return };
    let account = open_account(&daemon);
    let (handle, shell) = open_with(cx, options(&daemon));
    wait(cx, "the workspace", |cx| page(cx, &shell) == Page::Workspace);
    configure_app_lock(cx, handle, &shell, &account, 600_000);
    assert_eq!(page(cx, &shell), Page::Workspace);
    let locked = daemon
        .call(Some(account.as_str()), "account", "account.app_lock.lock", &serde_json::json!({}))
        .unwrap_or_else(|e| panic!("{e}"));
    println!("account.app_lock.lock answered: {locked}");
    let started = std::time::Instant::now();
    wait(cx, "the lock page", |cx| page(cx, &shell) == Page::Lock);
    println!("the lock page showed {:?} after the control client's lock", started.elapsed());
}
