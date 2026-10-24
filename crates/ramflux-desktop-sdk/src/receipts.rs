// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Delivery receipts for received direct messages (interim).
//!
//! The daemon sends a delivery receipt only when a client asks for one
//! (`message.receipt.delivered`); until it does that by itself, the desktop
//! asks once per direct message a peer sent, after the message is in the
//! account's read model (it was listed by `message.list`). Never for this
//! account's own messages, nor for copies from its other devices. Read
//! receipts are a separate privacy setting and are not sent here.
//!
//! - The request names the conversation, the message, this device as the
//!   receiver, and the sender's device and delivery target (from the
//!   identity directory), so the daemon also sends the receipt to the
//!   sender. It carries no timestamp (the daemon takes its own) and a retry
//!   key derived from the conversation and message: a resend, even after a
//!   restart, is the same operation and the daemon answers it from its
//!   receipt instead of sending twice.
//! - Receipts sent are remembered per daemon socket and account in the
//!   desktop settings file (ids only, the most recent [`REMEMBERED`]), so a
//!   restart or a backfill does not send again; every message listed that
//!   is not remembered is sent, which covers what arrived while the
//!   desktop was closed.
//! - A failed request is retried with a doubling back-off (from
//!   [`FIRST_RETRY`] up to [`LAST_RETRY`]); nothing waits for it.
//!
//! A sender device whose delivery target is not listed yet is skipped until
//! a later listing names it. When the daemon sends delivery receipts itself
//! this request becomes an idempotent no-op; drop this module then.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::time::Duration;

use ramflux_desktop_core::AccountId;
use serde_json::json;
use tokio::time::Instant;

use crate::bus::BusRequest;
use crate::local::settings_file;

/// The daemon method.
pub const METHOD: &str = "message.receipt.delivered";

/// How many sent receipts are remembered per account.
pub const REMEMBERED: usize = 4096;

/// The first retry after a failure.
pub const FIRST_RETRY: Duration = Duration::from_secs(2);

/// The longest wait between retries.
pub const LAST_RETRY: Duration = Duration::from_secs(300);

/// A received direct message to acknowledge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivered {
    /// Conversation id.
    pub conversation_id: String,
    /// Message id (the envelope id the sender chose).
    pub message_id: String,
    /// This device.
    pub receiver_device_id: String,
    /// The sender's device.
    pub sender_device_id: String,
    /// The sender device's delivery target.
    pub sender_target: String,
}

impl Delivered {
    /// The dedupe key: conversation and message.
    pub fn key(&self) -> String {
        format!("{}\n{}", self.conversation_id, self.message_id)
    }

    /// The request, with its stable retry key.
    pub fn request(&self, account: &str) -> BusRequest {
        let body = json!({
            "conversation_id": self.conversation_id,
            "message_id": self.message_id,
            "receiver_device_id": self.receiver_device_id,
            "recipient_device_id": self.sender_device_id,
            "target_delivery_id": self.sender_target,
        });
        let digest = ramflux_protocol::hash_hex(
            "ramflux.desktop.delivery_receipt.v1",
            self.key().as_bytes(),
        );
        BusRequest::new(Some(account), "message", METHOD, body)
            .with_operation_id(format!("desk-rcpt-{}", &digest[..32]))
    }
}

/// One receipt waiting to be sent.
#[derive(Debug, Clone)]
struct Pending {
    delivered: Delivered,
    attempts: u32,
    due: Instant,
    in_flight: bool,
}

/// The receipts of one account: what was sent, what waits.
#[derive(Debug)]
pub struct Receipts {
    path: Option<PathBuf>,
    socket: String,
    account: AccountId,
    sent: BTreeSet<String>,
    order: VecDeque<String>,
    pending: BTreeMap<String, Pending>,
}

impl Receipts {
    /// The receipts of `account` on the daemon at `socket`, remembered at
    /// `path` (in memory only when `None`).
    pub fn new(path: Option<PathBuf>, socket: impl Into<String>, account: AccountId) -> Self {
        let socket = socket.into();
        let order: VecDeque<String> = path
            .as_ref()
            .and_then(|path| settings_file::read(path))
            .and_then(|mut s| s.delivery_receipts.remove(&socket)?.remove(&account))
            .unwrap_or_default()
            .into();
        let sent = order.iter().cloned().collect();
        Self { path, socket, account, sent, order, pending: BTreeMap::new() }
    }

    /// Whether the receipt for `key` was sent.
    pub fn was_sent(&self, key: &str) -> bool {
        self.sent.contains(key)
    }

    /// Queues the listed messages that were not acknowledged yet.
    pub fn observe(&mut self, listed: Vec<Delivered>, now: Instant) {
        for delivered in listed {
            let key = delivered.key();
            if self.sent.contains(&key) || self.pending.contains_key(&key) {
                continue;
            }
            self.pending
                .insert(key, Pending { delivered, attempts: 0, due: now, in_flight: false });
        }
    }

    /// The receipts to send now (each marked in flight).
    pub fn due(&mut self, now: Instant) -> Vec<Delivered> {
        self.pending
            .values_mut()
            .filter(|p| !p.in_flight && p.due <= now)
            .map(|p| {
                p.in_flight = true;
                p.delivered.clone()
            })
            .collect()
    }

    /// When the next waiting receipt is due, if any.
    pub fn next_due(&self) -> Option<Instant> {
        self.pending.values().filter(|p| !p.in_flight).map(|p| p.due).min()
    }

    /// The answer to the receipt `key`: remembered when sent, retried
    /// later when not.
    pub fn answered(&mut self, key: &str, sent: bool, now: Instant) {
        if sent {
            self.pending.remove(key);
            self.remember(key);
            return;
        }
        if let Some(pending) = self.pending.get_mut(key) {
            pending.in_flight = false;
            pending.attempts = pending.attempts.saturating_add(1);
            let factor = 1_u32 << pending.attempts.saturating_sub(1).min(16);
            pending.due = now + FIRST_RETRY.saturating_mul(factor).min(LAST_RETRY);
        }
    }

    fn remember(&mut self, key: &str) {
        if !self.sent.insert(key.to_owned()) {
            return;
        }
        self.order.push_back(key.to_owned());
        while self.order.len() > REMEMBERED {
            if let Some(old) = self.order.pop_front() {
                self.sent.remove(&old);
            }
        }
        let Some(path) = &self.path else { return };
        let keys: Vec<String> = self.order.iter().cloned().collect();
        let _ = settings_file::update(path, |settings| {
            settings
                .delivery_receipts
                .entry(self.socket.clone())
                .or_default()
                .insert(self.account.clone(), keys);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delivered(message: &str) -> Delivered {
        Delivered {
            conversation_id: "dm-1".into(),
            message_id: message.into(),
            receiver_device_id: "device-me".into(),
            sender_device_id: "device-ana".into(),
            sender_target: "target-ana".into(),
        }
    }

    #[test]
    fn a_receipt_is_sent_once_and_remembered_across_restarts() {
        let dir = std::env::temp_dir().join(format!("rf-receipts-{}", std::process::id()));
        let path = dir.join("desktop-settings.json");
        let account = AccountId::new("acct-main");
        let now = Instant::now();
        let mut receipts = Receipts::new(Some(path.clone()), "rfd.sock", account.clone());
        receipts.observe(vec![delivered("m-1"), delivered("m-1")], now);
        let due = receipts.due(now);
        assert_eq!(due.len(), 1, "one per message");
        assert!(receipts.due(now).is_empty(), "nothing twice while in flight");
        let request = due[0].request("acct-main");
        assert_eq!(request.method, METHOD);
        assert_eq!(request.body["recipient_device_id"], "device-ana");
        assert_eq!(request.body["target_delivery_id"], "target-ana");
        assert!(request.body.get("delivered_at").is_none(), "the same body on every retry");
        let again = delivered("m-1").request("acct-main");
        assert_eq!(request.operation_id, again.operation_id, "a stable retry key");
        receipts.answered(&due[0].key(), true, now);
        receipts.observe(vec![delivered("m-1")], now);
        assert!(receipts.due(now).is_empty(), "a listed message is not acknowledged again");

        // A restart remembers it; a new message is sent.
        let mut restarted = Receipts::new(Some(path), "rfd.sock", account);
        assert!(restarted.was_sent(&delivered("m-1").key()));
        restarted.observe(vec![delivered("m-1"), delivered("m-2")], now);
        let due = restarted.due(now);
        assert_eq!(due.iter().map(|d| d.message_id.as_str()).collect::<Vec<_>>(), ["m-2"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_receipt_is_retried_with_a_growing_back_off() {
        let now = Instant::now();
        let mut receipts = Receipts::new(None, "rfd.sock", AccountId::new("a"));
        receipts.observe(vec![delivered("m-1")], now);
        let key = receipts.due(now)[0].key();
        receipts.answered(&key, false, now);
        assert!(receipts.due(now).is_empty(), "not before the back-off");
        assert_eq!(receipts.next_due(), Some(now + FIRST_RETRY));
        assert_eq!(receipts.due(now + FIRST_RETRY).len(), 1);
        receipts.answered(&key, false, now);
        assert_eq!(receipts.next_due(), Some(now + FIRST_RETRY * 2));
        for _ in 0..20 {
            let _ = receipts.due(now + LAST_RETRY * 2);
            receipts.answered(&key, false, now);
        }
        assert_eq!(receipts.next_due(), Some(now + LAST_RETRY), "the back-off is bounded");
    }
}
