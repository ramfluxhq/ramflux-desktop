// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Persisted dock layout: the full dock area (every split, tab group, the
//! active tab, sizes and which docks are open), the pinned tool and the dock
//! width, under a schema version.
//!
//! This is view-local preference, never domain state; it lives in a small
//! JSON file beside the application's other local settings. Without a path
//! (tests, screenshots) it lives in memory only. A file written under another
//! schema keeps only what is still meaningful (pinned tool and width); its
//! dock tree is dropped and the default layout is used.

use std::path::PathBuf;

use gpui_kit::component::dock::DockAreaState;
use serde::{Deserialize, Serialize};

use super::DockTool;

/// Width of the right dock when nothing is persisted. A physical layout seed
/// for the dock engine, which sizes docks in pixels; users resize freely.
pub const DEFAULT_DOCK_WIDTH: f32 = 420.;

/// Schema of the persisted layout. Bump it when the panels or their
/// arrangement change incompatibly; older dock trees are then ignored.
pub const LAYOUT_SCHEMA: u32 = 2;

fn legacy_schema() -> u32 {
    1
}

/// The persisted layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DockPrefs {
    /// Schema the file was written with (files before versioning are 1).
    #[serde(default = "legacy_schema")]
    pub schema: u32,
    /// Tool that stays open and reopens on launch.
    pub pinned: Option<DockTool>,
    /// Right dock width in pixels.
    pub width: f32,
    /// The whole dock area as the dock engine dumps it.
    #[serde(default)]
    pub layout: Option<DockAreaState>,
}

impl Default for DockPrefs {
    fn default() -> Self {
        Self { schema: LAYOUT_SCHEMA, pinned: None, width: DEFAULT_DOCK_WIDTH, layout: None }
    }
}

impl DockPrefs {
    /// Brings a loaded file to the current schema: the dock tree of another
    /// schema is dropped; pinned tool and width are kept.
    #[must_use]
    pub fn migrated(mut self) -> Self {
        if self.schema != LAYOUT_SCHEMA {
            self.layout = None;
            self.schema = LAYOUT_SCHEMA;
        }
        self
    }
}

/// Where the desktop keeps its local settings files (dock layout, desktop
/// settings): `RAMFLUX_DESKTOP_STATE_DIR`, else the user's configuration
/// directory.
pub use ramflux_desktop_sdk::local::paths::state_dir;

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

    /// The platform location: [`state_dir`]`/dock-layout.json`.
    pub fn default_location() -> Self {
        Self { path: state_dir().map(|dir| dir.join("dock-layout.json")) }
    }

    /// The stored layout, or the default when missing or unreadable.
    pub fn load(&self) -> DockPrefs {
        self.path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str::<DockPrefs>(&text).ok())
            .map(DockPrefs::migrated)
            .unwrap_or_default()
    }

    /// Stores the layout; failures are ignored (preferences are best effort).
    pub fn save(&self, prefs: &DockPrefs) {
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(prefs) {
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
        let prefs = DockPrefs {
            pinned: Some(DockTool::Approvals),
            width: 412.,
            layout: Some(DockAreaState { version: Some(2), ..Default::default() }),
            ..DockPrefs::default()
        };
        store.save(&prefs);
        assert_eq!(store.load(), prefs);
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(PrefsStore::in_memory().load(), DockPrefs::default());
    }

    #[test]
    fn a_file_from_another_schema_keeps_pin_and_width_but_not_the_tree() {
        let dir = std::env::temp_dir().join(format!("ramflux-prefs-old-{}", std::process::id()));
        let path = dir.join("dock-layout.json");
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(&path, r#"{"pinned":"Objects","width":380.0}"#).expect("legacy file");
        let loaded = PrefsStore::at(path).load();
        assert_eq!(loaded.schema, LAYOUT_SCHEMA);
        assert_eq!(loaded.pinned, Some(DockTool::Objects));
        assert!((loaded.width - 380.).abs() < f32::EPSILON);
        assert!(loaded.layout.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
