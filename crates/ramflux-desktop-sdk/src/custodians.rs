// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Which person's account custodies each AI participant this desktop made,
//! remembered across restarts.
//!
//! The daemon's account list names an AI participant's kind but not its
//! custodian, and the desktop store is not persisted, so the link the
//! creation ceremony made would be lost at the next start. It is kept here
//! (AI participant, custodian), per daemon socket: two account ids and
//! nothing else, never a credential, a phrase or a path. The daemon stays
//! the authority: it refuses an export from any account that is not the
//! custodian whatever this file says.

use std::collections::BTreeMap;
use std::path::PathBuf;

use ramflux_desktop_core::AccountId;

use crate::local::settings_file;

/// The custodian links of one daemon.
#[derive(Debug)]
pub struct CustodianStore {
    path: Option<PathBuf>,
    socket: String,
    links: BTreeMap<AccountId, AccountId>,
}

impl CustodianStore {
    /// The links of the daemon at `socket`, remembered at `path` (in memory
    /// only when `None`).
    pub fn new(path: Option<PathBuf>, socket: impl Into<String>) -> Self {
        let socket = socket.into();
        let links = path
            .as_ref()
            .and_then(|path| settings_file::read(path))
            .and_then(|mut settings| settings.agent_custodians.remove(&socket))
            .unwrap_or_default();
        Self { path, socket, links }
    }

    /// Every link remembered: AI participant to custodian.
    pub fn links(&self) -> BTreeMap<AccountId, AccountId> {
        self.links.clone()
    }

    /// Remembers that `custodian` custodies `agent` and writes the file.
    /// Best effort: a failed write only loses the link after a restart.
    pub fn remember(&mut self, agent: AccountId, custodian: AccountId) {
        if self.links.get(&agent) == Some(&custodian) {
            return;
        }
        self.links.insert(agent, custodian);
        let Some(path) = &self.path else { return };
        let links = self.links.clone();
        let _ = settings_file::update(path, |settings| {
            settings.agent_custodians.insert(self.socket.clone(), links);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_is_remembered_per_daemon_across_restarts() {
        let dir = std::env::temp_dir().join(format!("rf-custodians-{}", std::process::id()));
        let path = dir.join("desktop-settings.json");
        let (bot, owner) = (AccountId::new("acct-bot"), AccountId::new("acct-main"));
        let mut store = CustodianStore::new(Some(path.clone()), "/tmp/rfd.sock");
        store.remember(bot.clone(), owner.clone());
        let again = CustodianStore::new(Some(path.clone()), "/tmp/rfd.sock");
        assert_eq!(again.links().get(&bot), Some(&owner));
        let other = CustodianStore::new(Some(path.clone()), "/tmp/other.sock");
        assert!(other.links().is_empty(), "another daemon has its own accounts");
        let text = std::fs::read_to_string(&path).expect("file");
        assert!(text.contains("acct-bot") && !text.contains("passphrase"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
