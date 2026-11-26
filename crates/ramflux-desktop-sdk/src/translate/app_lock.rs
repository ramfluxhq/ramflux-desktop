// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The app lock of an account: the daemon's state reads and answers, the
//! unlock that clears it, and the refusal that means the lock holds the
//! account closed.

use ramflux_desktop_core::{AccountId, AppLock, AppLockState, DomainEvent};
use serde_json::Value;

use super::Translator;

/// The app lock a daemon body reports (`account.app_lock.state` and the
/// answers of its configure and lifecycle calls). Fails closed: `armed` is
/// unlocked, `unavailable` is unavailable, and any other or missing state is
/// locked.
pub fn app_lock_of(body: &Value) -> AppLock {
    let state = match body["state"].as_str() {
        Some("unlocked" | "armed") => AppLockState::Unlocked,
        Some("unavailable") => AppLockState::Unavailable,
        _ => AppLockState::Locked,
    };
    AppLock {
        enabled: body["enabled"].as_bool().unwrap_or(false),
        timeout_ms: body["timeout_ms"].as_u64().unwrap_or(0),
        state,
        reason: body["reason"].as_str().map(str::to_owned),
        generation: body["generation"].as_u64().unwrap_or(0),
    }
}

impl Translator {
    /// The app lock an answer or read reports for `account`.
    pub(super) fn app_lock_answer(
        &mut self,
        account: &AccountId,
        answer: &Value,
    ) -> Vec<DomainEvent> {
        self.set_app_lock(account, app_lock_of(answer))
    }

    /// The app lock read failed (not a refusal): the account stays closed
    /// until its account code opens it, and the next read corrects it.
    pub fn app_lock_unreadable(&mut self, account: &AccountId) -> Vec<DomainEvent> {
        let mut lock = self.app_locks.get(account).cloned().unwrap_or_default();
        lock.enabled = true;
        lock.state = AppLockState::Locked;
        lock.reason = Some("unreadable".to_owned());
        self.set_app_lock(account, lock)
    }

    /// The account code opened the account, which the daemon takes as the
    /// app lock's unlock as well. Nothing is known about an unread lock; its
    /// read (queued with the unlock) reports it.
    pub(super) fn app_unlocked(&mut self, account: &AccountId) -> Vec<DomainEvent> {
        let Some(mut lock) = self.app_locks.get(account).cloned() else { return Vec::new() };
        if lock.state != AppLockState::Locked {
            return Vec::new();
        }
        lock.state = AppLockState::Unlocked;
        lock.reason = Some("authenticated".to_owned());
        self.set_app_lock(account, lock)
    }

    /// The daemon refused a request because the app lock holds `account`
    /// closed: the lock screen takes over, and no error is shown for it. A
    /// refusal only comes from an account whose lock is on.
    pub fn app_locked(&mut self, account: &AccountId) -> Vec<DomainEvent> {
        let mut lock = self.app_locks.get(account).cloned().unwrap_or_default();
        lock.enabled = true;
        lock.state = AppLockState::Locked;
        self.set_app_lock(account, lock)
    }

    fn set_app_lock(&mut self, account: &AccountId, lock: AppLock) -> Vec<DomainEvent> {
        self.app_locks.insert(account.clone(), lock.clone());
        vec![DomainEvent::AppLockChanged { account_id: account.clone(), lock }]
    }
}
