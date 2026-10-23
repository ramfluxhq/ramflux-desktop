// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Where the desktop keeps its files on this machine.
//!
//! | What | macOS | Linux |
//! |---|---|---|
//! | settings (state) | `~/Library/Application Support/Ramflux Desktop` | `$XDG_CONFIG_HOME/ramflux-desktop` |
//! | logs | `~/Library/Logs/Ramflux Desktop` | `$XDG_STATE_HOME/ramflux-desktop/logs` |
//! | daemon data (default) | `<state>/rfd-data` | `<state>/rfd-data` |
//!
//! `RAMFLUX_DESKTOP_STATE_DIR` and `RAMFLUX_DESKTOP_LOG_DIR` override the
//! first two (tests, side-by-side installs). Nothing here is created; the
//! writers create what they need.

use std::path::PathBuf;

/// Overrides the state directory.
pub const STATE_DIR_ENV: &str = "RAMFLUX_DESKTOP_STATE_DIR";

/// Overrides the log directory.
pub const LOG_DIR_ENV: &str = "RAMFLUX_DESKTOP_LOG_DIR";

/// File name of the desktop settings file in the state directory.
pub const SETTINGS_FILE: &str = "desktop-settings.json";

/// File name of a daemon binary shipped inside the application bundle,
/// next to the desktop executable.
pub const BUNDLED_DAEMON: &str = "rf";

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// The directory of the desktop's local settings files.
pub fn state_dir() -> Option<PathBuf> {
    std::env::var_os(STATE_DIR_ENV).map(PathBuf::from).or_else(|| {
        let home = home()?;
        Some(if cfg!(target_os = "macos") {
            home.join("Library/Application Support/Ramflux Desktop")
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .map_or_else(|| home.join(".config"), PathBuf::from)
                .join("ramflux-desktop")
        })
    })
}

/// The desktop settings file.
pub fn settings_file() -> Option<PathBuf> {
    state_dir().map(|dir| dir.join(SETTINGS_FILE))
}

/// The directory of the application log (and the log of a daemon the
/// desktop started).
pub fn log_dir() -> Option<PathBuf> {
    std::env::var_os(LOG_DIR_ENV).map(PathBuf::from).or_else(|| {
        let home = home()?;
        Some(if cfg!(target_os = "macos") {
            home.join("Library/Logs/Ramflux Desktop")
        } else {
            std::env::var_os("XDG_STATE_HOME")
                .map_or_else(|| home.join(".local/state"), PathBuf::from)
                .join("ramflux-desktop/logs")
        })
    })
}

/// Where a daemon the desktop starts keeps its data when no other folder
/// is set: inside the state directory, never in a temporary folder.
pub fn default_data_dir() -> Option<PathBuf> {
    state_dir().map(|dir| dir.join("rfd-data"))
}

/// A daemon binary shipped next to the desktop executable (inside a
/// packaged application), if there is one.
pub fn bundled_daemon() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let candidate = exe.parent()?.join(BUNDLED_DAEMON);
    candidate.is_file().then_some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_dir_lives_under_the_state_dir() {
        if let (Some(state), Some(data)) = (state_dir(), default_data_dir()) {
            assert!(data.starts_with(state));
        }
        if let (Some(state), Some(file)) = (state_dir(), settings_file()) {
            assert_eq!(file.parent(), Some(state.as_path()));
        }
    }
}
