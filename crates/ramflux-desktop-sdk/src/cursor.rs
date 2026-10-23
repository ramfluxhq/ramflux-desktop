// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The event-stream cursor of each account, persisted in the desktop
//! settings file per daemon socket.
//!
//! The cursor is the sequence number of the last stream entry the source
//! applied: a number, never content. After a restart or an unlock the
//! source resumes from it, so the daemon replays only what happened in
//! between. A cursor the daemon no longer covers (older than what it keeps,
//! or newer than its head, for example after its data was replaced) is
//! refused with `resync_required` and the source reloads, so a stale
//! cursor costs a reload, never a missed change.
//!
//! Without a path (tests) the cursor lives in memory only.

use std::path::PathBuf;

use ramflux_desktop_core::AccountId;

use crate::local::settings_file;

/// Loads and saves one daemon's stream cursors.
#[derive(Debug, Clone, Default)]
pub struct CursorStore {
    path: Option<PathBuf>,
    socket: String,
}

impl CursorStore {
    /// Cursors of the daemon on `socket`, persisted at `path`, or kept
    /// nowhere when `None`.
    pub fn new(path: Option<PathBuf>, socket: impl Into<String>) -> Self {
        Self { path, socket: socket.into() }
    }

    /// The cursor of `account`, if one was saved.
    pub fn load(&self, account: &AccountId) -> Option<u64> {
        let mut settings = settings_file::read(self.path.as_ref()?)?;
        settings.stream_cursors.remove(&self.socket)?.remove(account)
    }

    /// Stores the cursor of `account`, keeping the rest of the file. Best
    /// effort: a failed write leaves the previous cursor, which only means
    /// a longer replay.
    pub fn save(&self, account: &AccountId, seq: u64) {
        let Some(path) = &self.path else { return };
        let _ = settings_file::update(path, |settings| {
            settings
                .stream_cursors
                .entry(self.socket.clone())
                .or_default()
                .insert(account.clone(), seq);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_are_kept_per_daemon_and_account() {
        let dir = std::env::temp_dir().join(format!("rf-cursor-{}", std::process::id()));
        let path = dir.join("desktop-settings.json");
        let one = CursorStore::new(Some(path.clone()), "/tmp/one.sock");
        let other = CursorStore::new(Some(path.clone()), "/tmp/other.sock");
        let (a, b) = (AccountId::new("a"), AccountId::new("b"));
        assert_eq!(one.load(&a), None);
        one.save(&a, 41);
        one.save(&b, 7);
        other.save(&a, 3);
        one.save(&a, 42);
        assert_eq!(one.load(&a), Some(42));
        assert_eq!(one.load(&b), Some(7));
        assert_eq!(other.load(&a), Some(3), "another daemon's journal has its own numbers");
        let text = std::fs::read_to_string(&path).expect("file");
        assert!(text.contains("stream_cursors"), "{text}");
        let _ = std::fs::remove_dir_all(dir);
        let memory = CursorStore::new(None, "/tmp/one.sock");
        memory.save(&a, 1);
        assert_eq!(memory.load(&a), None);
    }
}
