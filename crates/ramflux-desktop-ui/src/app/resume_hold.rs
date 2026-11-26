// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The content of an account stays covered after the window came back to the
//! front until the daemon has answered the latest foreground report (or has
//! closed the app lock). Each hold counts one report not answered yet, so an
//! answer to an older report cannot release a newer hold.

use std::collections::BTreeMap;

use ramflux_desktop_core::AccountId;

/// Foreground reports not answered yet, per account.
#[derive(Debug, Default)]
pub(super) struct ResumeHold {
    pending: BTreeMap<AccountId, u32>,
}

impl ResumeHold {
    /// A foreground report is about to be sent for `account`.
    pub(super) fn hold(&mut self, account: AccountId) {
        *self.pending.entry(account).or_default() += 1;
    }

    /// `answers` foreground reports of `account` were answered; whether that
    /// released the account.
    pub(super) fn answered(&mut self, account: &AccountId, answers: u32) -> bool {
        let Some(pending) = self.pending.get_mut(account) else { return false };
        *pending = pending.saturating_sub(answers);
        if *pending == 0 {
            self.pending.remove(account);
            return true;
        }
        false
    }

    /// The app lock of `account` is closed: the lock page covers it, and
    /// nothing is left to wait for.
    pub(super) fn closed(&mut self, account: &AccountId) -> bool {
        self.pending.remove(account).is_some()
    }

    /// Whether `account` is held.
    pub(super) fn holds(&self, account: &AccountId) -> bool {
        self.pending.contains_key(account)
    }

    /// Whether any account is held.
    pub(super) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_answer_to_the_latest_foreground_report_releases() {
        let account = AccountId::new("a");
        let mut hold = ResumeHold::default();
        // Two quick returns: two reports sent.
        hold.hold(account.clone());
        hold.hold(account.clone());
        assert!(!hold.answered(&account, 1), "the first answer leaves the newer report open");
        assert!(hold.holds(&account));
        assert!(hold.answered(&account, 1));
        assert!(hold.is_empty());
    }

    #[test]
    fn an_answer_with_nothing_held_changes_nothing() {
        let mut hold = ResumeHold::default();
        assert!(!hold.answered(&AccountId::new("a"), 1));
        assert!(hold.is_empty());
    }

    #[test]
    fn a_closed_lock_releases_whatever_is_pending() {
        let account = AccountId::new("a");
        let mut hold = ResumeHold::default();
        hold.hold(account.clone());
        hold.hold(account.clone());
        assert!(hold.closed(&account));
        assert!(hold.is_empty());
        assert!(!hold.closed(&account));
    }
}
