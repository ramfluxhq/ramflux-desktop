// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Persisted dock layout: which tool is pinned and the dock width.
//!
//! This is view-local preference, never domain state; it lives in a small
//! JSON file beside the application's other local settings. Without a path
//! (tests, screenshots) it lives in memory only.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::DockTool;

/// Width of the right dock when nothing is persisted. A physical layout seed
/// for the dock engine, which sizes docks in pixels; users resize freely.
pub const DEFAULT_DOCK_WIDTH: f32 = 420.;

/// The persisted layout.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DockPrefs {
    /// Tool that stays open and reopens on launch.
    pub pinned: Option<DockTool>,
    /// Right dock width in pixels.
    pub width: f32,
}

impl Default for DockPrefs {
    fn default() -> Self {
        Self { pinned: None, width: DEFAULT_DOCK_WIDTH }
    }
}

/// Loads and saves [`DockPrefs`].
#[derive(Debug, Clone, Default)]
pub struct PrefsStore {
    path: Option<PathBuf>,
}

impl PrefsStore {
    /// Memory only.
    pub fn in_memory() -> Self {
        Self { path: None }
    }

    /// Persisted at `path`.
    pub fn at(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    /// The platform location: `RAMFLUX_DESKTOP_STATE_DIR`, else the user's
    /// configuration directory.
    pub fn default_location() -> Self {
        let dir = std::env::var_os("RAMFLUX_DESKTOP_STATE_DIR").map(PathBuf::from).or_else(|| {
            let home = std::env::var_os("HOME").map(PathBuf::from)?;
            Some(if cfg!(target_os = "macos") {
                home.join("Library/Application Support/Ramflux Desktop")
            } else {
                std::env::var_os("XDG_CONFIG_HOME")
                    .map_or_else(|| home.join(".config"), PathBuf::from)
                    .join("ramflux-desktop")
            })
        });
        Self { path: dir.map(|dir| dir.join("dock-layout.json")) }
    }

    /// The stored layout, or the default when missing or unreadable.
    pub fn load(&self) -> DockPrefs {
        self.path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Stores the layout; failures are ignored (preferences are best effort).
    pub fn save(&self, prefs: DockPrefs) {
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&prefs) {
            let _ = std::fs::write(path, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_round_trip_through_a_file() {
        let dir = std::env::temp_dir().join(format!("ramflux-prefs-{}", std::process::id()));
        let store = PrefsStore::at(dir.join("dock-layout.json"));
        assert_eq!(store.load(), DockPrefs::default());
        let prefs = DockPrefs { pinned: Some(DockTool::Approvals), width: 412. };
        store.save(prefs);
        assert_eq!(store.load(), prefs);
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(PrefsStore::in_memory().load(), DockPrefs::default());
    }
}
