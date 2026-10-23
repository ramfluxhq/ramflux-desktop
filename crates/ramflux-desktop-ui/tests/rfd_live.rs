// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The production window root, headless, on a real `rf daemon` from the
//! pinned product revision (source = rfd), without a node: the entry page
//! shows the daemon's status and the explicit empty node list, a stopped
//! daemon sends it back to the daemon step, and a restarted one is picked
//! up again.
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
    let config = SdkConfig {
        request_timeout: Duration::from_secs(10),
        poll_fast: Duration::from_millis(100),
        poll_slow: Duration::from_millis(400),
        reconnect_backoff: Duration::from_millis(200),
        ..SdkConfig::new(daemon.socket())
    };
    ShellOptions {
        source: SourceChoice::Rfd(config),
        scenario: String::new(),
        start_clock: true,
        wall_clock: false,
        prefs: PrefsStore::in_memory(),
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
