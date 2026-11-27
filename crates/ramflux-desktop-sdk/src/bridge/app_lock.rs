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

use ramflux_desktop_core::{AccountId, AppLockCodeRefusal, DomainEvent, SecretInput};
use serde_json::Value;
use tokio::time::Instant;

use super::scope::Task;
use crate::bus::attended::CodeRequest;
use crate::translate::Scope;

/// Added to the daemon's remaining time so the read lands after the lock.
pub(super) const MARGIN: Duration = Duration::from_millis(250);

/// The longest wait between two repeated reads of an unreadable lock (and
/// between two tries to open a stream the lock refuses).
const BACKOFF_CAP: Duration = Duration::from_secs(30);

/// The wait before the next try to open a stream the app lock refused:
/// `base`, doubled for every refusal in a row, up to a cap.
pub(super) fn stream_backoff(base: Duration, streak: u32) -> Duration {
    base.saturating_mul(1u32 << streak.min(5)).min(BACKOFF_CAP.max(base))
}

/// The account code of the app lock change in flight (in memory only).
#[derive(Default)]
pub(super) struct AppLockCode {
    /// The account whose configure is in flight.
    account: Option<AccountId>,
    /// The daemon's prompt waiting for the window's answer.
    reply: Option<std::sync::mpsc::SyncSender<SecretInput>>,
    /// The window dismissed the sheet during the configure in flight.
    cancelled: bool,
}

impl AppLockCode {
    /// A configure of `account` starts: its code prompt, if any, is its own.
    pub(super) fn begin(&mut self, account: Option<AccountId>) {
        *self = Self { account, ..Self::default() };
    }

    /// The configure ended: a prompt still waiting is dropped (refused).
    pub(super) fn finish(&mut self) {
        *self = Self::default();
    }
}

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
    /// The daemon said the app lock of an account changed (on the account
    /// stream, or on a request connection attached to the account). A lock
    /// that is closed takes over at once; in any case the lock is read again
    /// and the daemon session lists the accounts again, so the stores hold
    /// what the daemon holds. `account` is the account the frame named, if it
    /// did; an account session only follows its own account.
    pub(super) fn app_lock_changed(&mut self, account: Option<&str>, body: &Value) {
        let own = self.account();
        let named = account.map(str::to_owned).or_else(|| own.clone());
        let Some(named) = named else { return };
        if own.as_ref().is_some_and(|own| *own != named) {
            return;
        }
        let account = AccountId::new(named);
        if body["state"].as_str() == Some("locked") {
            let closed = self.translator.app_locked(&account);
            self.emit(closed);
        } else if self.sync.task.is_none() && self.sync.open_at.is_some() {
            // Opened again: the stream the lock refused is tried at once.
            self.sync.open_at = Some(Instant::now());
        }
        self.sync.locked_streak = 0;
        self.request_reads(vec![crate::plan::fetch::Fetch::AppLock(account)]);
        self.refresh_daemon();
    }

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

    /// The daemon asks the window for the account code of the configure in
    /// flight. The sheet opens; the prompt waits for the answer (the provider
    /// refuses after its own bounded wait). With no configure in flight the
    /// request is dropped, which refuses it.
    pub(super) fn code_requested(&mut self, request: CodeRequest) {
        let Some(account) = self.app_lock_code.account.clone() else { return };
        self.app_lock_code.reply = Some(request.reply);
        self.app_lock_code.cancelled = false;
        self.emit(vec![DomainEvent::AppLockCodeSheet { account_id: account, open: true }]);
    }

    /// The window answers the sheet: a non-empty code goes to the prompt in
    /// flight; an empty one dismisses it. The secret is dropped (zeroized)
    /// here whether or not a prompt waits.
    pub(super) fn answer_code(&mut self, secrets: Vec<SecretInput>) {
        let secret = secrets.into_iter().next().unwrap_or_else(|| SecretInput::new(String::new()));
        if secret.is_empty() {
            self.app_lock_code.cancelled = true;
        }
        if let Some(reply) = self.app_lock_code.reply.take() {
            let _ = reply.send(secret);
        }
    }

    /// The refusal of an account code, as the window shows it. A refused
    /// prompt is a cancellation when the window dismissed the sheet.
    pub(super) fn code_refusal(&self, refusal: AppLockCodeRefusal) -> AppLockCodeRefusal {
        if refusal == AppLockCodeRefusal::TimedOut && self.app_lock_code.cancelled {
            AppLockCodeRefusal::Cancelled
        } else {
            refusal
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
