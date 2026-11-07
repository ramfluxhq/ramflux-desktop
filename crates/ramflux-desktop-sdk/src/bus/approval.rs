// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The schema gate's approval, enforced where requests are transmitted.
//!
//! The approval names one connection: the generation of the connection on
//! which the daemon's schema status was read and found compatible. Every
//! message-class request checks it after its connection is chosen and before
//! its frame is written, so a reconnect (or a keyed retry that reconnects)
//! can never carry a message ahead of the new connection's schema check.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The code a refused transmission reports.
pub const GATE_CLOSED: &str = "SchemaGateClosed";

/// Whether a daemon method is message-class: it reads or writes
/// conversations, messages, history, surfaces, tasks, calls' records or
/// objects.
#[must_use]
pub fn is_message_method(method: &str) -> bool {
    ["message.", "conversation.", "group.", "surface.", "task.", "object."]
        .iter()
        .any(|prefix| method.starts_with(prefix))
        || method == "call.list"
}

/// The shared approval of one scope's request connection.
#[derive(Debug, Clone, Default)]
pub struct Approval {
    inner: Arc<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// Whether the approval is enforced (account scopes).
    enforced: std::sync::atomic::AtomicBool,
    /// The approved connection generation; 0 is none.
    approved: AtomicU64,
}

impl Approval {
    /// Starts enforcing: nothing message-class is sent until a connection is
    /// approved.
    pub fn enforce(&self) {
        self.inner.enforced.store(true, Ordering::SeqCst);
    }

    /// Approves messages on the connection of `generation`.
    pub fn approve(&self, generation: u64) {
        self.inner.approved.store(generation, Ordering::SeqCst);
    }

    /// Approves messages on every connection (fixtures that never attach).
    #[cfg(test)]
    pub fn approve_any(&self) {
        self.inner.approved.store(u64::MAX, Ordering::SeqCst);
    }

    /// Withdraws the approval.
    pub fn revoke(&self) {
        self.inner.approved.store(0, Ordering::SeqCst);
    }

    /// Whether `method` may be transmitted on the connection of `generation`.
    #[must_use]
    pub fn permits(&self, method: &str, generation: u64) -> bool {
        !self.inner.enforced.load(Ordering::SeqCst)
            || !is_message_method(method)
            || self.inner.approved.load(Ordering::SeqCst) == generation
            || (cfg!(test) && self.inner.approved.load(Ordering::SeqCst) == u64::MAX)
    }
}
