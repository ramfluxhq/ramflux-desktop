// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Which backend failures a settings section still shows.
//!
//! The store keeps a bounded log of failed effects that is never cleared.
//! A section shows only failures of its own methods recorded after its mark
//! for the rendered account. A mark moves to the current failure count when
//! the section is shown, when the account changes, when the source comes
//! back after a disconnect, and when the section's own data arrives without
//! a new failure in the same change (a successful read or mutation).

use std::collections::BTreeMap;

use ramflux_desktop_core::{AccountId, EffectFailure, Listing, SourceStatus, Store};

use super::SettingsSection;
use crate::app::ChangeSet;

/// Contract method prefixes a section owns.
pub(crate) const fn prefixes(section: SettingsSection) -> &'static [&'static str] {
    match section {
        SettingsSection::Account => &["account."],
        SettingsSection::Backup => &["backup."],
        SettingsSection::Devices => &["device."],
        SettingsSection::Nodes => &["node."],
        SettingsSection::Agents => &["agent.", "identity."],
        SettingsSection::Integrations
        | SettingsSection::Diagnostics
        | SettingsSection::Appearance => &[],
    }
}

/// The method whose failure decides the devices directory state.
pub(crate) const DEVICE_LIST: &str = "device.list";

/// Failure marks per account and section, plus which accounts had their
/// device directory answered while their own session was rendered.
#[derive(Debug, Default)]
pub(crate) struct ErrorScope {
    marks: BTreeMap<(Option<AccountId>, SettingsSection), u64>,
    seen_total: u64,
    last_status: Option<SourceStatus>,
    last_account: Option<AccountId>,
    devices_listed: std::collections::BTreeSet<AccountId>,
}

impl ErrorScope {
    /// Moves the mark of `section` for `account` to the current count.
    pub(crate) fn clear(
        &mut self,
        account: Option<&AccountId>,
        section: SettingsSection,
        store: &Store,
    ) {
        self.marks.insert((account.cloned(), section), store.failures_total());
    }

    fn clear_all(&mut self, account: Option<&AccountId>, store: &Store) {
        for section in SettingsSection::ALL {
            self.clear(account, section, store);
        }
    }

    /// Follows one store change of the rendered account.
    pub(crate) fn observe(
        &mut self,
        account: Option<&AccountId>,
        changes: &ChangeSet,
        store: &Store,
    ) {
        let total = store.failures_total();
        let previous = std::mem::replace(&mut self.seen_total, total);
        let status = store.source_status();
        let last_status = self.last_status.replace(status);
        // `store` is the rendered account's own session store, so its
        // listing flag belongs to that account alone.
        if let Some(account) = account {
            self.note_listed(account, store);
        }
        if self.last_account.as_ref() != account {
            // Another account's failures never show under this one; its
            // store also counts failures on its own.
            self.last_account = account.cloned();
            self.clear_all(account, store);
            return;
        }
        if status == SourceStatus::Connected
            && last_status.is_some_and(|last| last != SourceStatus::Connected)
        {
            // Failures from before the reconnect are stale; a failure that
            // arrived in the same change is kept.
            for section in SettingsSection::ALL {
                self.marks.insert((account.cloned(), section), previous);
            }
        }
        if total != previous {
            return;
        }
        let domain = changes.domain();
        let succeeded = [
            (SettingsSection::Devices, !domain.devices.is_empty() || domain.listings),
            (SettingsSection::Account, !domain.accounts.is_empty()),
            (SettingsSection::Backup, !domain.backup_jobs.is_empty()),
            (SettingsSection::Nodes, !domain.nodes.is_empty()),
            (SettingsSection::Agents, !domain.agent_exports.is_empty()),
        ];
        for (section, changed) in succeeded {
            if changed {
                self.clear(account, section, store);
            }
        }
    }

    /// Records that `account`'s own session has listed its devices (its
    /// store is that account's, so the flag cannot leak to another one).
    pub(crate) fn note_listed(&mut self, account: &AccountId, store: &Store) {
        if store.is_listed(Listing::Devices) {
            self.devices_listed.insert(account.clone());
        }
    }

    /// Whether `account`'s device directory has been answered.
    pub(crate) fn devices_listed(&self, account: Option<&AccountId>) -> bool {
        account.is_some_and(|account| self.devices_listed.contains(account))
    }

    /// The newest current failure of `section` for `account`.
    pub(crate) fn current<'a>(
        &self,
        account: Option<&AccountId>,
        section: SettingsSection,
        store: &'a Store,
    ) -> Option<&'a EffectFailure> {
        let owned = prefixes(section);
        self.since(account, section, store)
            .iter()
            .rev()
            .find(|failure| owned.iter().any(|prefix| failure.method.starts_with(prefix)))
    }

    /// The newest current failure of exactly `method` within `section`.
    pub(crate) fn current_method<'a>(
        &self,
        account: Option<&AccountId>,
        section: SettingsSection,
        method: &str,
        store: &'a Store,
    ) -> Option<&'a EffectFailure> {
        self.since(account, section, store).iter().rev().find(|failure| failure.method == method)
    }

    fn since<'a>(
        &self,
        account: Option<&AccountId>,
        section: SettingsSection,
        store: &'a Store,
    ) -> &'a [EffectFailure] {
        let mark = self.marks.get(&(account.cloned(), section)).copied().unwrap_or(0);
        store.failures_since(mark)
    }
}
