// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! When the app lock of the open account is read again by the bridge itself.
//!
//! The daemon locks an account on its own clock, and an unfocused window
//! receives nothing on its ordinary connections. Two reads therefore come
//! from the bridge: one at the time the daemon said the lock is due (plus a
//! small margin), and a repeated one, with growing backoff, after a read that
//! could not be answered (the lock stays closed meanwhile).

use std::collections::BTreeMap;
use std::time::Duration;

use ramflux_desktop_core::AccountId;
use serde_json::Value;
use tokio::time::Instant;

use super::scope::Task;
use crate::translate::Scope;

/// Added to the daemon's remaining time so the read lands after the lock.
pub(super) const MARGIN: Duration = Duration::from_millis(250);

/// The longest wait between two repeated reads of an unreadable lock.
const BACKOFF_CAP: Duration = Duration::from_secs(30);

/// The next read of each account's app lock, if one is planned.
#[derive(Debug, Default)]
pub(super) struct AppLockWatch {
    plans: BTreeMap<AccountId, Plan>,
}

#[derive(Debug, Default)]
struct Plan {
    due: Option<Instant>,
    failures: u32,
}

impl AppLockWatch {
    /// Plans the next read of `account` from a lock the daemon reported: at
    /// its remaining time when it is enabled and armed (counting down from a
    /// report that the window went away); otherwise nothing is due.
    pub(super) fn observe(&mut self, account: &AccountId, body: &Value, now: Instant) {
        let due = match body["remaining_ms"].as_u64() {
            Some(ms)
                if ms > 0
                    && body["enabled"].as_bool() == Some(true)
                    && body["state"].as_str() == Some("armed") =>
            {
                Some(now + Duration::from_millis(ms) + MARGIN)
            }
            _ => None,
        };
        match due {
            Some(due) => {
                self.plans.insert(account.clone(), Plan { due: Some(due), failures: 0 });
            }
            None => {
                self.plans.remove(account);
            }
        }
    }

    /// Plans a repeated read of `account` after an unreadable one: `base`,
    /// doubled for every failure in a row, up to a cap.
    pub(super) fn unreadable(&mut self, account: &AccountId, base: Duration, now: Instant) {
        let plan = self.plans.entry(account.clone()).or_default();
        let doubled = base.saturating_mul(1u32 << plan.failures.min(5));
        plan.failures = plan.failures.saturating_add(1);
        plan.due = Some(now + doubled.min(BACKOFF_CAP.max(base)));
    }

    /// Drops every plan (the account closed or the daemon is gone).
    pub(super) fn clear(&mut self) {
        self.plans.clear();
    }

    /// When the earliest read is due.
    pub(super) fn next_due(&self) -> Option<Instant> {
        self.plans.values().filter_map(|plan| plan.due).min()
    }

    /// The accounts whose read is due at `now`; each is taken (planned again
    /// by the answer).
    pub(super) fn take_due(&mut self, now: Instant) -> Vec<AccountId> {
        let mut due = Vec::new();
        for (account, plan) in &mut self.plans {
            if plan.due.is_some_and(|at| now >= at) {
                plan.due = None;
                due.push(account.clone());
            }
        }
        due
    }
}

impl Task {
    /// The daemon refused a request of `account` because the app lock holds
    /// it closed. The lock screen is routed from the daemon's store, so an
    /// account session also has the daemon list and read the locks again; the
    /// daemon session's own refusal is already its store's.
    pub(super) fn app_lock_refused(&mut self, account: Option<AccountId>) {
        if let Some(account) = account {
            let closed = self.translator.app_locked(&account);
            self.emit(closed);
        }
        if self.scope != Scope::Daemon {
            self.refresh_daemon();
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn id(name: &str) -> AccountId {
        AccountId::new(name)
    }

    #[test]
    fn an_armed_lock_is_read_after_its_remaining_time() {
        let now = Instant::now();
        let mut watch = AppLockWatch::default();
        let a = id("a");
        watch.observe(&a, &json!({"enabled": true, "state": "armed", "remaining_ms": 4_000}), now);
        assert_eq!(watch.next_due(), Some(now + Duration::from_millis(4_000) + MARGIN));
        assert!(watch.take_due(now).is_empty());
        assert_eq!(watch.take_due(now + Duration::from_secs(5)), vec![a]);
        assert_eq!(watch.next_due(), None);
    }

    #[test]
    fn a_lock_that_is_not_armed_plans_nothing() {
        let now = Instant::now();
        let mut watch = AppLockWatch::default();
        let a = id("a");
        for body in [
            json!({"enabled": false, "state": "armed", "remaining_ms": 4_000}),
            json!({"enabled": true, "state": "locked", "remaining_ms": 0}),
            // An open account with no deadline: nothing to wait for.
            json!({"enabled": true, "state": "unlocked", "remaining_ms": 0}),
            json!({"enabled": true, "state": "unlocked", "remaining_ms": 4_000}),
            json!({"enabled": true, "state": "armed", "remaining_ms": 0}),
            json!({"enabled": true, "state": "armed"}),
        ] {
            watch.observe(&a, &body, now);
            assert_eq!(watch.next_due(), None, "{body}");
        }
    }

    #[test]
    fn every_account_has_its_own_due_time() {
        let now = Instant::now();
        let mut watch = AppLockWatch::default();
        let (a, b) = (id("a"), id("b"));
        watch.observe(&a, &json!({"enabled": true, "state": "armed", "remaining_ms": 1_000}), now);
        watch.observe(&b, &json!({"enabled": true, "state": "armed", "remaining_ms": 9_000}), now);
        assert_eq!(watch.take_due(now + Duration::from_secs(2)), vec![a.clone()]);
        assert_eq!(watch.next_due(), Some(now + Duration::from_millis(9_000) + MARGIN));
        // A later answer of one account leaves the other's plan alone.
        watch.observe(&a, &json!({"enabled": true, "state": "unlocked", "remaining_ms": 0}), now);
        assert_eq!(watch.take_due(now + Duration::from_secs(10)), vec![b]);
    }

    #[test]
    fn an_unreadable_lock_is_read_again_with_growing_backoff() {
        let now = Instant::now();
        let base = Duration::from_secs(1);
        let mut watch = AppLockWatch::default();
        let a = id("a");
        let mut waits = Vec::new();
        for _ in 0..8 {
            watch.unreadable(&a, base, now);
            waits.push(watch.next_due().map(|at| at - now));
        }
        assert_eq!(waits[0], Some(Duration::from_secs(1)));
        assert_eq!(waits[1], Some(Duration::from_secs(2)));
        assert_eq!(waits[2], Some(Duration::from_secs(4)));
        assert_eq!(waits[7], Some(Duration::from_secs(30)));
        watch.observe(&a, &json!({"enabled": true, "state": "locked"}), now);
        watch.unreadable(&a, base, now);
        assert_eq!(watch.next_due(), Some(now + base));
    }
}
