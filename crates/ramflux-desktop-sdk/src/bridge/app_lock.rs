// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! When the app lock of the open account is read again by the bridge itself.
//!
//! The daemon locks an account on its own clock, and an unfocused window
//! receives nothing on its ordinary connections. Two reads therefore come
//! from the bridge: one at the time the daemon said the lock is due (plus a
//! small margin), and a repeated one, with growing backoff, after a read that
//! could not be answered (the lock stays closed meanwhile).

use std::time::Duration;

use serde_json::Value;
use tokio::time::Instant;

/// Added to the daemon's remaining time so the read lands after the lock.
pub(super) const MARGIN: Duration = Duration::from_millis(250);

/// The longest wait between two repeated reads of an unreadable lock.
const BACKOFF_CAP: Duration = Duration::from_secs(30);

/// The next read of the app lock, if one is planned.
#[derive(Debug, Default)]
pub(super) struct AppLockWatch {
    due: Option<Instant>,
    failures: u32,
}

impl AppLockWatch {
    /// Plans the next read from a lock the daemon reported: at its remaining
    /// time when it is enabled, unlocked and counting down; otherwise nothing
    /// is due.
    pub(super) fn observe(&mut self, body: &Value, now: Instant) {
        self.failures = 0;
        let counting = body["enabled"].as_bool() == Some(true)
            && matches!(body["state"].as_str(), Some("unlocked" | "armed"));
        self.due = match (counting, body["remaining_ms"].as_u64()) {
            (true, Some(ms)) => Some(now + Duration::from_millis(ms) + MARGIN),
            _ => None,
        };
    }

    /// Plans a repeated read after an unreadable one: `base`, doubled for
    /// every failure in a row, up to a cap.
    pub(super) fn unreadable(&mut self, base: Duration, now: Instant) {
        let doubled = base.saturating_mul(1u32 << self.failures.min(5));
        self.failures = self.failures.saturating_add(1);
        self.due = Some(now + doubled.min(BACKOFF_CAP.max(base)));
    }

    /// Drops the plan (the account closed or the daemon is gone).
    pub(super) fn clear(&mut self) {
        self.due = None;
        self.failures = 0;
    }

    /// When the next read is due.
    pub(super) fn next_due(&self) -> Option<Instant> {
        self.due
    }

    /// Whether a read is due at `now`; it is taken (planned again by the
    /// answer).
    pub(super) fn take_due(&mut self, now: Instant) -> bool {
        if self.due.is_some_and(|at| now >= at) {
            self.due = None;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_counting_lock_is_read_after_its_remaining_time() {
        let now = Instant::now();
        let mut watch = AppLockWatch::default();
        watch.observe(&json!({"enabled": true, "state": "unlocked", "remaining_ms": 4_000}), now);
        assert_eq!(watch.next_due(), Some(now + Duration::from_millis(4_000) + MARGIN));
        assert!(!watch.take_due(now));
        assert!(watch.take_due(now + Duration::from_secs(5)));
        assert_eq!(watch.next_due(), None);
    }

    #[test]
    fn a_lock_that_is_not_counting_plans_nothing() {
        let now = Instant::now();
        let mut watch = AppLockWatch::default();
        for body in [
            json!({"enabled": false, "state": "unlocked", "remaining_ms": 4_000}),
            json!({"enabled": true, "state": "locked", "remaining_ms": 0}),
            json!({"enabled": true, "state": "unlocked"}),
        ] {
            watch.observe(&body, now);
            assert_eq!(watch.next_due(), None);
        }
    }

    #[test]
    fn an_unreadable_lock_is_read_again_with_growing_backoff() {
        let now = Instant::now();
        let base = Duration::from_secs(1);
        let mut watch = AppLockWatch::default();
        let mut waits = Vec::new();
        for _ in 0..8 {
            watch.unreadable(base, now);
            waits.push(watch.next_due().map(|at| at - now));
        }
        assert_eq!(waits[0], Some(Duration::from_secs(1)));
        assert_eq!(waits[1], Some(Duration::from_secs(2)));
        assert_eq!(waits[2], Some(Duration::from_secs(4)));
        assert_eq!(waits[7], Some(Duration::from_secs(30)));
        watch.observe(&json!({"enabled": true, "state": "locked"}), now);
        watch.unreadable(base, now);
        assert_eq!(watch.next_due(), Some(now + base));
    }
}
