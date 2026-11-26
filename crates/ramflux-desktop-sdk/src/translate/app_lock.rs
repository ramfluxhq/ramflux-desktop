// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The app lock of an account: the daemon's state reads and answers, and the
//! refusal that means the lock holds the account closed.

use ramflux_desktop_core::{AccountId, AppLock, AppLockState, DomainEvent};
use serde_json::Value;

use super::Translator;

/// The app lock a daemon body reports (`account.app_lock.state` and the
/// answers of its configure and lifecycle calls). `None` when the body names
/// no state.
pub fn app_lock_of(body: &Value) -> Option<AppLock> {
    let state = match body["state"].as_str()? {
        "locked" => AppLockState::Locked,
        "unavailable" => AppLockState::Unavailable,
        "unlocked" => AppLockState::Unlocked,
        _ => return None,
    };
    Some(AppLock {
        enabled: body["enabled"].as_bool().unwrap_or(false),
        timeout_ms: body["timeout_ms"].as_u64().unwrap_or(0),
        state,
        reason: body["reason"].as_str().map(str::to_owned),
        generation: body["generation"].as_u64().unwrap_or(0),
    })
}

impl Translator {
    /// The app lock an answer or read reports for `account`.
    pub(super) fn app_lock_answer(
        &mut self,
        account: &AccountId,
        answer: &Value,
    ) -> Vec<DomainEvent> {
        let Some(lock) = app_lock_of(answer) else { return Vec::new() };
        self.app_locks.insert(account.clone(), lock.clone());
        vec![DomainEvent::AppLockChanged { account_id: account.clone(), lock }]
    }

    /// The daemon refused a request because the app lock holds `account`
    /// closed: the lock screen takes over, and no error is shown for it. A
    /// refusal only comes from an account whose lock is on.
    pub fn app_locked(&mut self, account: &AccountId) -> Vec<DomainEvent> {
        let mut lock = self.app_locks.get(account).cloned().unwrap_or_default();
        lock.enabled = true;
        lock.state = AppLockState::Locked;
        self.app_locks.insert(account.clone(), lock.clone());
        vec![DomainEvent::AppLockChanged { account_id: account.clone(), lock }]
    }
}
