// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The daemon behind one desktop, and the reference CLI on the same socket.
//!
//! The desktop does everything it can itself. The reference CLI is used to
//! cross-check what the desktop shows and for the steps the desktop cannot
//! take yet (each use is recorded as a note in the step table):
//! - starting a direct conversation (the desktop sends only in a
//!   conversation the daemon already lists a route for);
//! - receiving a conversation the desktop does not know yet (the desktop
//!   receives only the conversations it shows).

use std::path::PathBuf;

use ramflux_desktop_test::live_daemon::LiveDaemon;
use serde_json::Value;

/// The first message of a direct conversation, sent with the reference CLI.
#[derive(Debug, Clone)]
pub struct DmSend {
    /// Local account that sends.
    pub account: String,
    /// Conversation id.
    pub conversation: String,
    /// Message id.
    pub message: String,
    /// Envelope id.
    pub envelope: String,
    /// The sender's principal.
    pub source_principal: String,
    /// The sender's device.
    pub sender: String,
    /// The recipient's principal commitment.
    pub recipient_commitment: String,
    /// The recipient's device.
    pub recipient_device: String,
    /// The recipient's delivery target.
    pub target: String,
    /// The text.
    pub body: String,
}

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

    /// Sends the first message of a direct conversation.
    ///
    /// # Errors
    ///
    /// The CLI's failure.
    fn dm_send(&self, send: &DmSend) -> Result<(), String>;

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

    fn dm_send(&self, send: &DmSend) -> Result<(), String> {
        rf_json(
            self,
            &[
                "dm",
                "send",
                "--account",
                &send.account,
                "--conversation",
                &send.conversation,
                "--message",
                &send.message,
                "--envelope",
                &send.envelope,
                "--source-principal",
                &send.source_principal,
                "--sender",
                &send.sender,
                "--recipient-principal-commitment",
                &send.recipient_commitment,
                "--recipient-device",
                &send.recipient_device,
                "--target",
                &send.target,
                "--body",
                &send.body,
            ],
        )
        .map(drop)
    }

    fn dm_read(&self, account: &str, conversation: &str) -> Result<(), String> {
        rf_json(self, &["dm", "read", "--account", account, "--conversation", conversation])
            .map(drop)
    }
}
