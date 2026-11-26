// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The content of an account stays covered after the window came back to the
//! front until the daemon has answered the latest foreground report (or has
//! closed the app lock). Each hold counts one report not answered yet, so an
//! answer to an older report cannot release a newer hold.

use std::collections::BTreeMap;

use ramflux_desktop_core::{AccountId, AppLockPhase};

/// Foreground reports not answered yet, per account, and the phase the
/// daemon last answered for each account.
#[derive(Debug, Default)]
pub(super) struct ResumeHold {
    pending: BTreeMap<AccountId, u32>,
    reported: BTreeMap<AccountId, AppLockPhase>,
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

    /// The daemon answered a report of `phase` for `account`: that is the
    /// phase it has now.
    pub(super) fn reported(&mut self, account: &AccountId, phase: AppLockPhase) {
        self.reported.insert(account.clone(), phase);
    }

    /// A lock answer shows `account` unlocked with no deadline: whatever
    /// reported the window as gone is no longer true in the daemon (another
    /// client unlocked it), so the next settle reports the phase again.
    pub(super) fn unarmed(&mut self, account: &AccountId) {
        if self.reported.get(account) == Some(&AppLockPhase::Background) {
            self.reported.remove(account);
        }
    }

    /// The phase the daemon last answered for `account`.
    pub(super) fn phase(&self, account: &AccountId) -> Option<AppLockPhase> {
        self.reported.get(account).copied()
    }

    /// Forgets the phases of accounts not in `keep`.
    pub(super) fn retain_phases(&mut self, keep: &[AccountId]) {
        self.reported.retain(|id, _| keep.contains(id));
    }

    /// The daemon connection was lost: every hold goes (the content is not
    /// kept covered for an answer that cannot come) and the phases the daemon
    /// had are unknown; whether anything was held.
    pub(super) fn lost(&mut self) -> bool {
        self.reported.clear();
        let held = !self.pending.is_empty();
        self.pending.clear();
        held
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
    fn a_lost_connection_drops_every_hold_and_phase() {
        let account = AccountId::new("a");
        let mut hold = ResumeHold::default();
        hold.hold(account.clone());
        hold.reported(&account, AppLockPhase::Background);
        assert!(hold.lost());
        assert!(hold.is_empty());
        assert_eq!(hold.phase(&account), None);
        assert!(!hold.lost());
    }

    #[test]
    fn an_unarmed_lock_forgets_only_a_background_phase() {
        let account = AccountId::new("a");
        let mut hold = ResumeHold::default();
        hold.reported(&account, AppLockPhase::Background);
        hold.unarmed(&account);
        assert_eq!(hold.phase(&account), None);
        hold.reported(&account, AppLockPhase::Foreground);
        hold.unarmed(&account);
        assert_eq!(hold.phase(&account), Some(AppLockPhase::Foreground));
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
