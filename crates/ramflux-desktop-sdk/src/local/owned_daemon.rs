// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The record of a daemon the desktop started and has not stopped yet.
//!
//! Written when a start succeeds and removed when that daemon stops or
//! exits. If the application ends without removing it (a crash), the next
//! start finds the record: a daemon answering on the recorded socket was
//! left behind by the desktop, and the user can stop it (`daemon.stop`). A
//! daemon the user started without the desktop leaves no record and is never
//! offered for stopping.
//!
//! The record is a small JSON file next to the settings file; it holds a
//! process id and a socket path only.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::paths;

/// File name of the record in the state directory.
pub const OWNED_DAEMON_FILE: &str = "owned-daemon.json";

/// A daemon the desktop started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedDaemon {
    /// Its process id.
    pub pid: u32,
    /// The socket it listens on.
    pub socket: PathBuf,
}

/// Where the record lives.
#[derive(Debug, Clone, Default)]
pub struct OwnedDaemonFile {
    path: Option<PathBuf>,
}

impl OwnedDaemonFile {
    /// Nowhere (tests, screenshots): nothing is recorded.
    pub fn in_memory() -> Self {
        Self { path: None }
    }

    /// At `path`.
    pub fn at(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    /// In the platform state directory.
    pub fn default_location() -> Self {
        Self { path: paths::state_dir().map(|dir| dir.join(OWNED_DAEMON_FILE)) }
    }

    /// The file, if any.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The recorded daemon, if a record exists and reads.
    pub fn load(&self) -> Option<OwnedDaemon> {
        let text = std::fs::read_to_string(self.path.as_deref()?).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Records `daemon` (best effort: without a record a crash leaves an
    /// unoffered daemon, nothing worse).
    pub fn record(&self, daemon: &OwnedDaemon) {
        let Some(path) = self.path.as_deref() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string(daemon) {
            let _ = std::fs::write(path, text);
        }
    }

    /// Removes the record.
    pub fn clear(&self) {
        if let Some(path) = self.path.as_deref() {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_survives_until_cleared() {
        let dir = std::env::temp_dir().join(format!("rf-owned-{}", std::process::id()));
        let file = OwnedDaemonFile::at(dir.join(OWNED_DAEMON_FILE));
        assert_eq!(file.load(), None);
        let daemon = OwnedDaemon { pid: 42, socket: PathBuf::from("/tmp/x.sock") };
        file.record(&daemon);
        assert_eq!(OwnedDaemonFile::at(dir.join(OWNED_DAEMON_FILE)).load(), Some(daemon));
        file.clear();
        assert_eq!(file.load(), None);
        assert_eq!(OwnedDaemonFile::in_memory().load(), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
