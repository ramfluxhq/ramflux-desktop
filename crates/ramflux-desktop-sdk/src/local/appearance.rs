// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Appearance choices kept in the desktop settings file: the interface
//! theme and language. Both are per-device preferences: they are never
//! synced to other devices and hold nothing secret. An unset choice
//! follows the operating system.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::paths;
use super::settings_file::{self, SettingsError};

/// The value that makes a choice follow the operating system.
pub const FOLLOW_SYSTEM: &str = "system";

/// The persisted appearance choices; `None` means "follow the system".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppearanceSettings {
    /// The id of the chosen theme (a token set's name), or
    /// [`FOLLOW_SYSTEM`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// The chosen interface language (a locale code such as `en`), or
    /// [`FOLLOW_SYSTEM`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

impl AppearanceSettings {
    /// Whether nothing is chosen (the file part can be left out).
    pub fn is_empty(&self) -> bool {
        self.theme.is_none() && self.language.is_none()
    }
}

/// Where the appearance choices are kept.
#[derive(Debug, Clone, Default)]
pub struct AppearanceStore {
    path: Option<PathBuf>,
}

impl AppearanceStore {
    /// Memory only (tests): loads nothing, saves nowhere.
    pub fn in_memory() -> Self {
        Self { path: None }
    }

    /// Persisted in the settings file at `path`.
    pub fn at(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    /// The platform settings file.
    pub fn default_location() -> Self {
        Self { path: paths::settings_file() }
    }

    /// The settings file, if any.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The stored choices, or none when missing or unreadable.
    pub fn load(&self) -> AppearanceSettings {
        self.path
            .as_deref()
            .and_then(settings_file::read)
            .map(|settings| settings.appearance)
            .unwrap_or_default()
    }

    /// Stores `appearance`, keeping every other part of the file.
    ///
    /// # Errors
    ///
    /// As [`settings_file::update`]. Without a path nothing is written and
    /// nothing fails.
    pub fn save(&self, appearance: &AppearanceSettings) -> Result<(), SettingsError> {
        let Some(path) = &self.path else { return Ok(()) };
        settings_file::update(path, |settings| settings.appearance = appearance.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rf-appearance-{name}-{}", std::process::id()))
    }

    #[test]
    fn choices_round_trip_and_keep_the_launch_part() {
        let dir = dir("round-trip");
        let path = dir.join("desktop-settings.json");
        settings_file::update(&path, |s| s.launch.data_dir = Some(PathBuf::from("/data")))
            .expect("test: launch part written");
        let store = AppearanceStore::at(path.clone());
        assert_eq!(store.load(), AppearanceSettings::default(), "nothing chosen yet");
        let chosen = AppearanceSettings {
            theme: Some("dark".to_owned()),
            language: Some(FOLLOW_SYSTEM.to_owned()),
        };
        store.save(&chosen).expect("test: appearance written");
        assert_eq!(store.load(), chosen, "the choices read back as written");
        let file = settings_file::read(&path).expect("test: file reads");
        assert_eq!(
            file.launch.data_dir.as_deref(),
            Some(Path::new("/data")),
            "saving the appearance must not drop another writer's part"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_in_memory_store_saves_nowhere() {
        let store = AppearanceStore::in_memory();
        let chosen = AppearanceSettings { theme: Some("light".to_owned()), language: None };
        assert_eq!(store.save(&chosen), Ok(()), "an in-memory store never fails");
        assert!(store.load().is_empty(), "an in-memory store keeps nothing");
        assert!(store.path().is_none());
    }

    #[test]
    fn a_file_without_appearance_reads_as_following_the_system() {
        let dir = dir("old");
        let path = dir.join("desktop-settings.json");
        std::fs::create_dir_all(&dir).expect("test: dir");
        std::fs::write(&path, r#"{"schema":1,"pinned":{}}"#).expect("test: file");
        assert!(AppearanceStore::at(path).load().is_empty(), "older files carry no choice");
        let _ = std::fs::remove_dir_all(dir);
    }
}
