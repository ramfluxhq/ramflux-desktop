// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Notification choices kept in the desktop settings file: how much of a
//! message an operating-system notification may show. A per-device
//! preference, never synced and holding nothing secret.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::paths;
use super::settings_file::{self, SettingsError};

/// How much a notification about a message, call or approval reveals.
///
/// The default is [`PreviewLevel::SenderOnly`]: the notification says who
/// wrote (useful enough to decide whether to look) but never what, so no
/// end-to-end encrypted text reaches the notification server, its on-screen
/// card or any history it keeps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewLevel {
    /// The sender's name and the text (or a short preview of it).
    SenderAndText,
    /// The sender's name only.
    #[default]
    SenderOnly,
    /// Neither: a generic notice that something arrived.
    Nothing,
}

impl PreviewLevel {
    /// Every level, from most to least revealing.
    pub const ALL: [Self; 3] = [Self::SenderAndText, Self::SenderOnly, Self::Nothing];
}

/// The persisted notification choices; `None` means the default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationSettings {
    /// The preview level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<PreviewLevel>,
}

impl NotificationSettings {
    /// Whether nothing is chosen (the file part can be left out).
    pub fn is_empty(&self) -> bool {
        self.preview.is_none()
    }

    /// The chosen level, else the default.
    pub fn preview_or_default(&self) -> PreviewLevel {
        self.preview.unwrap_or_default()
    }
}

/// Where the notification choices are kept.
#[derive(Debug, Clone, Default)]
pub struct NotificationStore {
    path: Option<PathBuf>,
}

impl NotificationStore {
    /// Memory only (tests): loads defaults, saves nowhere.
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

    /// The stored choices, or defaults when missing or unreadable.
    pub fn load(&self) -> NotificationSettings {
        self.path
            .as_deref()
            .and_then(settings_file::read)
            .map(|settings| settings.notifications)
            .unwrap_or_default()
    }

    /// Stores `notifications`, keeping every other part of the file.
    ///
    /// # Errors
    ///
    /// As [`settings_file::update`]. Without a path nothing is written and
    /// nothing fails.
    pub fn save(&self, notifications: &NotificationSettings) -> Result<(), SettingsError> {
        let Some(path) = &self.path else { return Ok(()) };
        settings_file::update(path, |settings| settings.notifications = notifications.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_shows_the_sender_only() {
        assert_eq!(NotificationSettings::default().preview_or_default(), PreviewLevel::SenderOnly);
    }

    #[test]
    fn choices_round_trip_and_keep_other_parts() {
        let dir = std::env::temp_dir().join(format!("rf-notify-{}", std::process::id()));
        let file = dir.join("desktop-settings.json");
        settings_file::update(&file, |s| s.appearance.theme = Some("dark".into())).expect("seed");
        let store = NotificationStore::at(file.clone());
        assert_eq!(store.load(), NotificationSettings::default());
        for level in PreviewLevel::ALL {
            let chosen = NotificationSettings { preview: Some(level) };
            store.save(&chosen).expect("save");
            assert_eq!(store.load(), chosen);
        }
        let text = std::fs::read_to_string(&file).expect("file");
        assert!(text.contains("\"preview\": \"nothing\""), "{text}");
        assert_eq!(
            settings_file::read(&file).and_then(|s| s.appearance.theme),
            Some("dark".into()),
            "other parts are kept"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
