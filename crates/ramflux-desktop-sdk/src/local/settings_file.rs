// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The desktop settings file: a small versioned JSON file holding desktop
//! preferences and local bookkeeping only (pinned contexts, launch choices,
//! event-stream cursors, ids of delivery receipts sent), never content,
//! keys or secrets.
//!
//! Every writer reads, changes its own part and replaces the file
//! atomically (write a sibling, then rename), so one writer never drops
//! another's part. A file written under a newer schema is left untouched:
//! it reads as empty and is never overwritten, so an older build cannot
//! destroy a newer one's state.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use ramflux_desktop_core::{AccountId, ContextId};
use serde::{Deserialize, Serialize};

use super::launch::LaunchSettings;

/// Schema of the settings file. Bump it on an incompatible change; new
/// optional fields with defaults do not need a bump.
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
    /// Source, socket, daemon binary and data directory choices.
    #[serde(default)]
    pub launch: LaunchSettings,
    /// The last event-stream entry applied, per daemon socket and account
    /// (a sequence number, not content).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stream_cursors: BTreeMap<String, BTreeMap<AccountId, u64>>,
    /// Delivery receipts sent, per daemon socket and account: conversation
    /// and message ids only (see [`crate::receipts`]).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub delivery_receipts: BTreeMap<String, BTreeMap<AccountId, Vec<String>>>,
    /// Authenticated public renames shown in conversations, per daemon
    /// socket and account (public names only, see [`crate::renames`]).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub name_changes: BTreeMap<String, BTreeMap<AccountId, Vec<crate::renames::RenameRecord>>>,
}

/// Why the settings file was not written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    /// The file was written by a newer build; it is kept as it is.
    #[error("the settings file was written by a newer version (schema {0})")]
    NewerSchema(u32),
    /// The file could not be written.
    #[error("cannot write the settings file: {0}")]
    Io(String),
}

/// The settings at `path` under the current schema; `None` when missing,
/// unreadable or written under another schema.
pub fn read(path: &Path) -> Option<DesktopSettings> {
    read_any(path).filter(|settings| settings.schema == SETTINGS_SCHEMA)
}

fn read_any(path: &Path) -> Option<DesktopSettings> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Reads the file at `path`, lets `change` edit it and writes it back
/// atomically. An unreadable or older file starts from defaults.
///
/// # Errors
///
/// [`SettingsError::NewerSchema`] when a newer build wrote the file (it
/// is left untouched), [`SettingsError::Io`] when it cannot be written.
pub fn update(path: &Path, change: impl FnOnce(&mut DesktopSettings)) -> Result<(), SettingsError> {
    let mut settings = match read_any(path) {
        Some(settings) if settings.schema > SETTINGS_SCHEMA => {
            return Err(SettingsError::NewerSchema(settings.schema));
        }
        Some(settings) if settings.schema == SETTINGS_SCHEMA => settings,
        _ => DesktopSettings { schema: SETTINGS_SCHEMA, ..DesktopSettings::default() },
    };
    change(&mut settings);
    let text =
        serde_json::to_string_pretty(&settings).map_err(|e| SettingsError::Io(e.to_string()))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| SettingsError::Io(e.to_string()))?;
    }
    // A unique sibling per write: two writers never share a partial file.
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let n = WRITES.fetch_add(1, Ordering::Relaxed);
    let partial = path.with_extension(format!("json.{}-{n}.partial", std::process::id()));
    std::fs::write(&partial, text).map_err(|e| SettingsError::Io(e.to_string()))?;
    std::fs::rename(&partial, path).map_err(|e| {
        let _ = std::fs::remove_file(&partial);
        SettingsError::Io(e.to_string())
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rf-settings-{name}-{}", std::process::id()))
    }

    #[test]
    fn writers_keep_each_others_parts() {
        let dir = dir("parts");
        let path = dir.join("desktop-settings.json");
        update(&path, |s| {
            s.pinned.insert(AccountId::new("a"), vec![ContextId::new("g-1")]);
        })
        .expect("pins");
        update(&path, |s| s.launch.socket_path = Some(PathBuf::from("/tmp/x.sock"))).expect("l");
        let settings = read(&path).expect("file");
        assert_eq!(settings.pinned.len(), 1);
        assert_eq!(settings.launch.socket_path.as_deref(), Some(Path::new("/tmp/x.sock")));
        let leftovers = std::fs::read_dir(&dir).expect("dir").count();
        assert_eq!(leftovers, 1, "no partial files are left behind");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_without_launch_choices_still_reads() {
        let dir = dir("old");
        let path = dir.join("desktop-settings.json");
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(&path, r#"{"schema":1,"pinned":{"a":["g-1"]}}"#).expect("file");
        let settings = read(&path).expect("file");
        assert_eq!(settings.launch, LaunchSettings::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_newer_file_is_refused_and_kept() {
        let dir = dir("newer");
        let path = dir.join("desktop-settings.json");
        std::fs::create_dir_all(&dir).expect("dir");
        let newer = r#"{"schema":2,"other":true}"#;
        std::fs::write(&path, newer).expect("file");
        assert_eq!(read(&path), None);
        assert_eq!(update(&path, |_| {}), Err(SettingsError::NewerSchema(2)));
        assert_eq!(std::fs::read_to_string(&path).ok().as_deref(), Some(newer));
        let _ = std::fs::remove_dir_all(dir);
    }
}
