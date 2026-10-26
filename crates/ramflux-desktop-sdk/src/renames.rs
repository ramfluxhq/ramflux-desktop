// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Authenticated public renames of an account's peers, remembered across
//! restarts.
//!
//! The daemon announces a rename once, on its event stream
//! (`contact.changed` with `reason: public_profile_renamed`). The desktop
//! store is not persisted and the stream resumes after the last entry
//! applied, so a rename line would vanish after a restart. Each rename is
//! kept here (principal, old and new public name, the stream entry's
//! sequence number and time), bounded, per daemon socket and account, and
//! shown again in the conversations of that principal once they are
//! listed. Public profile names only; never message content.

use std::collections::VecDeque;
use std::path::PathBuf;

use ramflux_desktop_core::AccountId;
use serde::{Deserialize, Serialize};

use crate::local::settings_file;

/// How many renames are remembered per account (the oldest go first).
pub const REMEMBERED: usize = 256;

/// One authenticated rename.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameRecord {
    /// The renamed principal.
    pub principal_id: String,
    /// The public name before.
    pub old_name: String,
    /// The public name now.
    pub new_name: String,
    /// The stream entry's sequence number (`None` from a daemon without a
    /// journal).
    #[serde(default)]
    pub seq: Option<u64>,
    /// When the daemon recorded it, in Unix seconds.
    pub at: i64,
}

impl RenameRecord {
    /// What tells two renames apart: the stream entry when known, else
    /// the rename itself.
    pub fn key(&self) -> String {
        match self.seq {
            Some(seq) => format!("s{seq}"),
            None => format!("n{}:{}:{}", self.principal_id, self.old_name, self.new_name),
        }
    }
}

/// The renames of one account.
#[derive(Debug)]
pub struct RenameStore {
    path: Option<PathBuf>,
    socket: String,
    account: AccountId,
    records: VecDeque<RenameRecord>,
}

impl RenameStore {
    /// The renames of `account` on the daemon at `socket`, remembered at
    /// `path` (in memory only when `None`).
    pub fn new(path: Option<PathBuf>, socket: impl Into<String>, account: AccountId) -> Self {
        let socket = socket.into();
        let records: VecDeque<RenameRecord> = path
            .as_ref()
            .and_then(|path| settings_file::read(path))
            .and_then(|mut s| s.name_changes.remove(&socket)?.remove(&account))
            .unwrap_or_default()
            .into();
        Self { path, socket, account, records }
    }

    /// The renames remembered, oldest first.
    pub fn records(&self) -> Vec<RenameRecord> {
        self.records.iter().cloned().collect()
    }

    /// Remembers `record` (once per key) and writes the file. Best effort:
    /// a failed write only loses the line after a restart.
    pub fn remember(&mut self, record: RenameRecord) {
        if self.records.iter().any(|r| r.key() == record.key()) {
            return;
        }
        self.records.push_back(record);
        while self.records.len() > REMEMBERED {
            self.records.pop_front();
        }
        let Some(path) = &self.path else { return };
        let records: Vec<RenameRecord> = self.records.iter().cloned().collect();
        let _ = settings_file::update(path, |settings| {
            settings
                .name_changes
                .entry(self.socket.clone())
                .or_default()
                .insert(self.account.clone(), records);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(seq: Option<u64>, new: &str) -> RenameRecord {
        RenameRecord {
            principal_id: "p-ana".into(),
            old_name: "Ana".into(),
            new_name: new.into(),
            seq,
            at: 1_790_000_000,
        }
    }

    #[test]
    fn renames_survive_a_restart_once_each_and_bounded() {
        let dir = std::env::temp_dir().join(format!("rf-renames-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("settings.json");
        let _ = std::fs::remove_file(&path);
        let account = AccountId::new("acct-main");
        let mut store = RenameStore::new(Some(path.clone()), "/tmp/rfd.sock", account.clone());
        store.remember(record(Some(7), "Ana R."));
        store.remember(record(Some(7), "Ana R."));
        // The same rename again later is another entry.
        store.remember(record(Some(9), "Ana R."));
        let again = RenameStore::new(Some(path.clone()), "/tmp/rfd.sock", account.clone());
        assert_eq!(again.records(), [record(Some(7), "Ana R."), record(Some(9), "Ana R.")]);
        let other = RenameStore::new(Some(path.clone()), "/tmp/other.sock", account);
        assert!(other.records().is_empty(), "per daemon socket");
        let mut bounded = RenameStore::new(None, "s", AccountId::new("a"));
        for seq in 0..(REMEMBERED as u64 + 3) {
            bounded.remember(record(Some(seq), "x"));
        }
        assert_eq!(bounded.records().len(), REMEMBERED);
        assert_eq!(bounded.records()[0].seq, Some(3), "the oldest go first");
        let _ = std::fs::remove_dir_all(dir);
    }
}
