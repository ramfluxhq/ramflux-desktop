// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Launch choices kept in the desktop settings file: the data source, the
//! daemon socket, the daemon binary the desktop may start, and the data
//! directory such a daemon uses. Unset choices fall back to defaults: the
//! build's source, the reference CLI's socket, a daemon binary bundled with
//! the application, and a data directory under the state directory.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::paths;
use super::settings_file::{self, SettingsError};
use crate::config::DEFAULT_SOCKET;

/// Where the data comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    /// The bundled demo scenarios.
    Mock,
    /// A local daemon over its socket.
    Rfd,
}

impl SourceKind {
    /// The build's default: scenarios in debug builds, the daemon in
    /// release builds.
    pub fn build_default() -> Self {
        if cfg!(debug_assertions) { Self::Mock } else { Self::Rfd }
    }

    /// Parses `mock` or `rfd`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "mock" => Some(Self::Mock),
            "rfd" => Some(Self::Rfd),
            _ => None,
        }
    }

    /// `mock` or `rfd`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::Rfd => "rfd",
        }
    }
}

/// The persisted launch choices; `None` means "use the default".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchSettings {
    /// Data source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceKind>,
    /// The daemon's local-bus socket.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket_path: Option<PathBuf>,
    /// The `rf` binary the desktop runs as `rf daemon start`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon_binary: Option<PathBuf>,
    /// Data directory of a daemon the desktop starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<PathBuf>,
    /// Whether to poll when the daemon offers no event stream (default: on).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_fallback: Option<bool>,
}

/// Which daemon binary a start would run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonBinary {
    /// Set in the settings.
    Configured(PathBuf),
    /// Shipped inside the application bundle.
    Bundled(PathBuf),
    /// None set and none bundled.
    Missing,
}

impl DaemonBinary {
    /// The binary's path, if there is one.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Configured(path) | Self::Bundled(path) => Some(path),
            Self::Missing => None,
        }
    }
}

impl LaunchSettings {
    /// The chosen source, else the build's default.
    pub fn source_or_default(&self) -> SourceKind {
        self.source.unwrap_or_else(SourceKind::build_default)
    }

    /// The chosen socket, else the reference CLI's default.
    pub fn socket_or_default(&self) -> PathBuf {
        self.socket_path.clone().unwrap_or_else(|| PathBuf::from(DEFAULT_SOCKET))
    }

    /// Whether polling stands in for a missing event stream (on unless
    /// turned off).
    pub fn poll_fallback_or_default(&self) -> bool {
        self.poll_fallback.unwrap_or(true)
    }

    /// The chosen data directory, else the default under the state
    /// directory.
    pub fn data_dir_or_default(&self) -> Option<PathBuf> {
        self.data_dir.clone().or_else(paths::default_data_dir)
    }

    /// The configured binary, else a bundled one.
    pub fn daemon_binary(&self) -> DaemonBinary {
        self.daemon_binary_with(paths::bundled_daemon())
    }

    /// The configured binary, else `bundled`.
    pub fn daemon_binary_with(&self, bundled: Option<PathBuf>) -> DaemonBinary {
        match (&self.daemon_binary, bundled) {
            (Some(path), _) => DaemonBinary::Configured(path.clone()),
            (None, Some(path)) => DaemonBinary::Bundled(path),
            (None, None) => DaemonBinary::Missing,
        }
    }
}

/// Loads and saves the [`LaunchSettings`] part of the settings file.
#[derive(Debug, Clone, Default)]
pub struct LaunchStore {
    path: Option<PathBuf>,
}

impl LaunchStore {
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
    pub fn load(&self) -> LaunchSettings {
        self.path
            .as_deref()
            .and_then(settings_file::read)
            .map(|settings| settings.launch)
            .unwrap_or_default()
    }

    /// Stores `launch`, keeping every other part of the file.
    ///
    /// # Errors
    ///
    /// As [`settings_file::update`]. Without a path nothing is written and
    /// nothing fails.
    pub fn save(&self, launch: &LaunchSettings) -> Result<(), SettingsError> {
        let Some(path) = &self.path else { return Ok(()) };
        settings_file::update(path, |settings| settings.launch = launch.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_choices_fall_back_to_defaults() {
        let launch = LaunchSettings::default();
        assert_eq!(launch.socket_or_default(), PathBuf::from(DEFAULT_SOCKET));
        assert_eq!(launch.source_or_default(), SourceKind::build_default());
        assert_eq!(launch.daemon_binary_with(None), DaemonBinary::Missing);
        let bundled = PathBuf::from("/Applications/R.app/Contents/MacOS/rf");
        assert_eq!(
            launch.daemon_binary_with(Some(bundled.clone())),
            DaemonBinary::Bundled(bundled.clone())
        );
        let configured = LaunchSettings {
            daemon_binary: Some(PathBuf::from("/opt/rf")),
            ..LaunchSettings::default()
        };
        assert_eq!(
            configured.daemon_binary_with(Some(bundled)),
            DaemonBinary::Configured(PathBuf::from("/opt/rf")),
            "a configured binary wins over a bundled one"
        );
    }

    #[test]
    fn choices_round_trip_through_the_file() {
        let dir = std::env::temp_dir().join(format!("rf-launch-{}", std::process::id()));
        let store = LaunchStore::at(dir.join("desktop-settings.json"));
        assert_eq!(store.load(), LaunchSettings::default());
        let launch = LaunchSettings {
            source: Some(SourceKind::Rfd),
            socket_path: Some(PathBuf::from("/tmp/rf/rfd.sock")),
            daemon_binary: Some(PathBuf::from("/opt/rf/bin/rf")),
            data_dir: Some(PathBuf::from("/tmp/rf/data")),
            poll_fallback: Some(false),
        };
        store.save(&launch).expect("save");
        assert_eq!(store.load(), launch);
        let text = std::fs::read_to_string(dir.join("desktop-settings.json")).expect("file");
        assert!(text.contains("\"source\": \"rfd\""), "{text}");
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(LaunchStore::in_memory().save(&launch), Ok(()));
        assert_eq!(LaunchStore::in_memory().load(), LaunchSettings::default());
    }

    #[test]
    fn sources_parse_from_their_names() {
        for kind in [SourceKind::Mock, SourceKind::Rfd] {
            assert_eq!(SourceKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(SourceKind::parse("daemon"), None);
    }
}
