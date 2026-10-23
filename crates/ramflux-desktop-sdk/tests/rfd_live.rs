// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The SDK source against a real `rf daemon` from the pinned product
//! revision, without a node: daemon status, the empty node list, the
//! account list, error mapping, diagnostics, and a stopped daemon that
//! disconnects, comes back and is re-snapshotted.
//!
//! Ignored by default. Run with a pinned `rf`:
//!
//! ```sh
//! RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=/path/to/bin/rf \
//!   cargo test -p ramflux-desktop-sdk --test rfd_live -- --ignored --test-threads=1
//! ```
//!
//! A lock and unlock round trip needs an account, and creating, unlocking
//! or restoring one opens a gateway session on a node, so it is not
//! exercised without a node; unlocking an account the daemon does not know
//! is checked as an error instead.

mod common;

use std::time::Duration;

use common::{Fed, config};
use ramflux_desktop_core::{
    AccountId, DomainEvent, Effect, FailureCode, ListState, Listing, LockState, SecretInput,
    Source as _, SourceStatus, methods,
};
use ramflux_desktop_sdk::SdkHub;
use ramflux_desktop_test::live_daemon::{self, LiveDaemon};
use serde_json::json;

/// The daemon for one test, or `None` (with a note) when live tests are off.
fn daemon(tag: &str) -> Option<LiveDaemon> {
    if !live_daemon::enabled() {
        println!("skipped: set {}=1 to run against a real daemon", live_daemon::ENABLE_ENV);
        return None;
    }
    match LiveDaemon::start(tag) {
        Ok(daemon) => Some(daemon),
        Err(error) => panic!("cannot start the daemon: {error}"),
    }
}

fn hub(daemon: &LiveDaemon) -> SdkHub {
    SdkHub::start(ramflux_desktop_sdk::SdkConfig {
        request_timeout: Duration::from_secs(10),
        ..config(&daemon.socket())
    })
    .expect("bridge")
}

#[test]
#[ignore = "needs a real rf daemon: RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=..."]
fn daemon_status_nodes_and_accounts_map_from_a_real_daemon() {
    let Some(daemon) = daemon("map") else { return };
    let hub = hub(&daemon);
    let mut entry = Fed::new(hub.daemon_source());
    entry.until("connected and listed", |s, _| {
        s.source_status() == SourceStatus::Connected
            && s.is_listed(Listing::Nodes)
            && s.is_listed(Listing::Accounts)
    });
    let info = entry.store.daemon().cloned().expect("daemon info");
    assert_eq!(info.socket_path, daemon.socket().to_string_lossy(), "the daemon's own socket");
    assert_eq!(info.open_accounts, 0);
    assert_eq!(entry.store.nodes().count(), 0, "a fresh daemon has no node");
    assert_eq!(
        ListState::of(entry.store.is_listed(Listing::Nodes), true),
        ListState::Empty,
        "an empty node list is an explicit empty state"
    );
    assert_eq!(entry.store.accounts().count(), 0, "no account is open");
    // The reference CLI agrees.
    let cli = daemon.rf(&["account", "list"]).expect("rf account list");
    let listed: serde_json::Value = serde_json::from_slice(&cli.stdout).expect("json");
    assert_eq!(listed["accounts"], json!([]));
    assert!(entry.store.rejections().is_empty(), "{:?}", entry.store.rejections());
    assert_eq!(entry.names()[0], "CapabilitiesUpdated");
}

#[test]
#[ignore = "needs a real rf daemon: RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=..."]
fn an_account_the_daemon_has_not_open_maps_to_locked_and_unlock_fails_cleanly() {
    let Some(daemon) = daemon("lock") else { return };
    let hub = hub(&daemon);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-none")));
    account.until("locked", |s, _| {
        s.account(&AccountId::new("acct-none")).is_some_and(|a| a.lock_state == LockState::Locked)
    });
    let mut entry = Fed::new(hub.daemon_source());
    entry.until("connected", |s, _| s.source_status() == SourceStatus::Connected);
    entry.source.submit(Effect::SecretCall {
        method: methods::ACCOUNT_UNLOCK.to_owned(),
        body: json!({ "account_id": "acct-none" }),
        secrets: vec![SecretInput::new("not a real passphrase")],
    });
    entry.until("unlock failed", |s, _| !s.failures().is_empty());
    let failure = entry.store.failures()[0].clone();
    println!("unlock of an unknown account: {failure:?}");
    assert_eq!(failure.method, methods::ACCOUNT_UNLOCK);
    assert!(
        matches!(failure.code, FailureCode::Daemon(_)),
        "a daemon refusal, not a transport error: {failure:?}"
    );
    assert!(!failure.message.contains("not a real passphrase"), "no secret in the message");
    assert_eq!(entry.store.source_status(), SourceStatus::Connected, "a refusal keeps the link");
}

#[test]
#[ignore = "needs a real rf daemon: RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=..."]
fn diagnostics_export_writes_a_redacted_bundle_from_the_real_daemon() {
    let Some(daemon) = daemon("diag") else { return };
    let hub = hub(&daemon);
    let mut entry = Fed::new(hub.daemon_source());
    entry.until("connected", |s, _| s.source_status() == SourceStatus::Connected);
    let path = daemon.dir().join("diagnostics.json");
    entry.source.submit(Effect::Call {
        method: methods::DIAGNOSTICS_EXPORT.to_owned(),
        body: json!({ "path": path.to_string_lossy() }),
    });
    entry.until("written", |_, _| path.exists());
    let bundle: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("bundle")).expect("json");
    assert_eq!(bundle["schema"], "ramflux.desktop.diagnostics.v1");
    assert_eq!(bundle["daemon"]["reachable"], true);
    assert_eq!(bundle["nodes"], json!([]));
}

#[test]
#[ignore = "needs a real rf daemon: RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=..."]
fn a_stopped_daemon_disconnects_and_a_restarted_one_is_resnapshotted() {
    let Some(mut daemon) = daemon("again") else { return };
    let hub = hub(&daemon);
    let mut entry = Fed::new(hub.daemon_source());
    entry.until("connected", |s, _| {
        s.source_status() == SourceStatus::Connected && s.is_listed(Listing::Nodes)
    });
    daemon.stop();
    assert!(!daemon.is_running());
    // The next poll finds the daemon gone.
    entry.until("disconnected", |s, _| s.source_status() == SourceStatus::Disconnected);
    let seq_at_loss = entry.store.last_seq();
    daemon.restart().expect("restart");
    entry.until("reconnected", |s, _| s.source_status() == SourceStatus::Connected);
    entry.until("re-snapshotted", |_, envelopes| {
        envelopes
            .iter()
            .rev()
            .take_while(|e| e.seq > seq_at_loss)
            .any(|e| matches!(&e.event, DomainEvent::DaemonInfoUpdated(_)))
    });
    assert!(entry.store.last_seq() > seq_at_loss, "the cursor keeps moving forward");
    assert_eq!(entry.source.last_seq(), entry.store.last_seq());
    assert!(entry.store.rejections().is_empty(), "{:?}", entry.store.rejections());
}
