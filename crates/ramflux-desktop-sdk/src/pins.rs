// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Pinned contexts, persisted per account in the desktop settings file.
//!
//! The daemon has no pin method, so pinning is a desktop-local preference.
//! The source keeps it in a small versioned JSON file and restores it after
//! the account's snapshot, so pins survive a restart. The file holds only
//! account and context ids in pin order; never content.
//!
//! A file written under a newer schema is left untouched (read as empty and
//! never overwritten), so an older build cannot destroy a newer one's state.
//! Without a path (tests) pins live in memory only.

use std::collections::BTreeMap;
use std::path::PathBuf;

use ramflux_desktop_core::{AccountId, ContextId};
use serde::{Deserialize, Serialize};

/// Schema of the settings file. Bump it on an incompatible change.
pub const SETTINGS_SCHEMA: u32 = 1;

fn legacy_schema() -> u32 {
    1
}

/// The persisted desktop settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopSettings {
    /// Schema the file was written with.
    #[serde(default = "legacy_schema")]
    pub schema: u32,
    /// Pinned contexts per account, in pin order.
    #[serde(default)]
    pub pinned: BTreeMap<AccountId, Vec<ContextId>>,
}

/// Loads and saves the pins of [`DesktopSettings`].
#[derive(Debug, Clone, Default)]
pub struct PinStore {
    path: Option<PathBuf>,
}

impl PinStore {
    /// Pins persisted at `path`, or kept nowhere when `None`.
    pub fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }

    fn read(&self) -> Option<DesktopSettings> {
        let text = std::fs::read_to_string(self.path.as_ref()?).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// The pins of `account` in pin order, or `None` when the desktop has
    /// never recorded any for it (missing, unreadable, or another schema).
    pub fn load(&self, account: &AccountId) -> Option<Vec<ContextId>> {
        self.read()
            .filter(|settings| settings.schema == SETTINGS_SCHEMA)
            .and_then(|mut settings| settings.pinned.remove(account))
    }

    /// Stores the pins of `account`, keeping every other account's. An empty
    /// list is kept too: "nothing pinned" is a choice. Best effort: a failed
    /// write leaves the previous file in place.
    pub fn save(&self, account: &AccountId, pins: &[ContextId]) {
        let Some(path) = &self.path else { return };
        let mut settings = match self.read() {
            Some(settings) if settings.schema > SETTINGS_SCHEMA => return,
            Some(settings) if settings.schema == SETTINGS_SCHEMA => settings,
            _ => DesktopSettings { schema: SETTINGS_SCHEMA, ..DesktopSettings::default() },
        };
        settings.pinned.insert(account.clone(), pins.to_vec());
        let Ok(text) = serde_json::to_string_pretty(&settings) else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let partial = path.with_extension("json.partial");
        if std::fs::write(&partial, text).is_ok() {
            let _ = std::fs::rename(&partial, path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rf-pins-{name}-{}", std::process::id()))
    }

    #[test]
    fn pins_round_trip_per_account_in_order() {
        let dir = dir("trip");
        let store = PinStore::new(Some(dir.join("desktop-settings.json")));
        let (a, b) = (AccountId::new("acct-a"), AccountId::new("acct-b"));
        assert_eq!(store.load(&a), None);
        store.save(&a, &[ContextId::new("g-2"), ContextId::new("dm-1")]);
        store.save(&b, &[ContextId::new("g-9")]);
        assert_eq!(store.load(&a), Some(vec![ContextId::new("g-2"), ContextId::new("dm-1")]));
        assert_eq!(store.load(&b), Some(vec![ContextId::new("g-9")]));
        store.save(&a, &[]);
        assert_eq!(store.load(&a), Some(Vec::new()), "unpinning everything is remembered");
        assert_eq!(store.load(&b).map(|p| p.len()), Some(1), "other accounts are kept");
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(PinStore::new(None).load(&a), None);
    }

    #[test]
    fn a_newer_schema_is_neither_read_nor_overwritten() {
        let dir = dir("newer");
        let path = dir.join("desktop-settings.json");
        std::fs::create_dir_all(&dir).expect("dir");
        let newer = r#"{"schema":2,"pinned":{"acct-a":["g-1"]},"other":true}"#;
        std::fs::write(&path, newer).expect("file");
        let store = PinStore::new(Some(path.clone()));
        let account = AccountId::new("acct-a");
        assert_eq!(store.load(&account), None);
        store.save(&account, &[ContextId::new("g-2")]);
        assert_eq!(std::fs::read_to_string(&path).ok().as_deref(), Some(newer));
        let _ = std::fs::remove_dir_all(dir);
    }
}
