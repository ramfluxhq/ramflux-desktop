// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The daemon behind one desktop, and the reference CLI on the same socket.
//!
//! The desktop does everything it can itself, starting conversations and
//! sending attachments included. The reference CLI is used to cross-check
//! what the desktop shows and for the one step the daemon cannot take yet
//! (recorded as a note in the step table): receiving a conversation the
//! receiving desktop does not know yet. Until the daemon routes a first
//! message from an unknown conversation itself, `rf dm read` fetches it.

use std::path::PathBuf;

use ramflux_desktop_test::live_daemon::LiveDaemon;
use serde_json::Value;

/// A daemon one desktop is attached to.
pub trait DemoDaemon {
    /// Its local-bus socket.
    fn socket(&self) -> PathBuf;

    /// Stops it and starts it again on the same socket and data.
    ///
    /// # Errors
    ///
    /// Why it did not come back.
    fn restart(&mut self) -> Result<(), String>;

    /// `account status` of `account`.
    ///
    /// # Errors
    ///
    /// The CLI's failure.
    fn account_status(&self, account: &str) -> Result<Value, String>;

    /// The public key of the listed node `node`, if it is listed.
    ///
    /// # Errors
    ///
    /// The CLI's failure.
    fn node_public_key(&self, node: &str) -> Result<Option<String>, String>;

    /// Receives `conversation` for `account` from the node.
    ///
    /// # Errors
    ///
    /// The CLI's failure.
    fn dm_read(&self, account: &str, conversation: &str) -> Result<(), String>;
}

/// Runs the reference CLI on `daemon` and parses its JSON answer.
fn rf_json(daemon: &LiveDaemon, args: &[&str]) -> Result<Value, String> {
    let out = daemon.rf(args).map_err(|e| format!("cannot run rf: {e}"))?;
    if !out.status.success() {
        let verb = args.iter().take(2).copied().collect::<Vec<_>>().join(" ");
        return Err(format!(
            "rf {verb} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim().lines().last().unwrap_or("no output")
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("rf answered no JSON: {e}"))
}

impl DemoDaemon for LiveDaemon {
    fn socket(&self) -> PathBuf {
        LiveDaemon::socket(self)
    }

    fn restart(&mut self) -> Result<(), String> {
        LiveDaemon::restart(self)
    }

    fn account_status(&self, account: &str) -> Result<Value, String> {
        rf_json(self, &["account", "status", "--account", account])
    }

    fn node_public_key(&self, node: &str) -> Result<Option<String>, String> {
        let listed = rf_json(self, &["node", "list"])?;
        Ok(listed["nodes"].as_array().into_iter().flatten().find_map(|n| {
            (n["node_id"] == node).then(|| n["node_public_key"].as_str().map(str::to_owned))?
        }))
    }

    fn dm_read(&self, account: &str, conversation: &str) -> Result<(), String> {
        rf_json(self, &["dm", "read", "--account", account, "--conversation", conversation])
            .map(drop)
    }
}
