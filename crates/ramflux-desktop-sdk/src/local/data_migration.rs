// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The one-time copy of a started daemon's default data root from where an
//! older build kept it (`$XDG_CONFIG_HOME/ramflux-desktop/rfd-data`) to the
//! XDG data directory (`$XDG_DATA_HOME/ramflux-desktop/rfd-data`).
//!
//! Safety rules:
//!
//! 1. **Copy, verify, then switch.** The tree is copied to a staging folder
//!    beside the destination, every file is compared byte for byte with its
//!    source, and only then is the staging folder renamed into place. The
//!    rename is the switch: [`super::paths::choose_data_dir`] picks the new
//!    root once it exists and the old one until then.
//! 2. **Never delete user data.** The old folder is left exactly as it was;
//!    the user may remove it after checking. Only the desktop's own staging
//!    folder is ever removed (a partial copy from a failed or interrupted
//!    run).
//! 3. **Not while a daemon uses it.** The daemon holds an exclusive lock on
//!    `daemon.lock` in its data root for as long as it runs; the copy takes
//!    the same lock for its whole duration, so no daemon runs on the old
//!    root while it is read, and a running one makes the copy wait for a
//!    later start.
//! 4. **Idempotent.** Once the destination exists nothing happens; a run
//!    that stopped half way starts over from a clean staging folder.
//! 5. **Fail closed.** A symbolic link or special file in the tree, a
//!    mismatch or any I/O error abandons the copy; the old root stays the
//!    default.

use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use super::platform_dirs;

/// The lock file a daemon holds in its data root.
pub const DAEMON_LOCK: &str = "daemon.lock";

/// Suffix of the staging folder beside the destination.
const STAGING_SUFFIX: &str = "migrating";

/// Suffix of the record written beside the destination after the switch.
const RECORD_SUFFIX: &str = "migrated-from";

/// What a migration run did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// No old data root exists.
    NothingToMove,
    /// The new data root exists already; nothing was touched.
    AlreadyMigrated,
    /// A daemon holds the old data root; try again on a later start.
    InUse,
    /// Copied and verified, then switched.
    Migrated {
        /// Regular files copied.
        files: u64,
        /// Bytes copied.
        bytes: u64,
    },
}

/// Why a migration was abandoned (the old data root stays the default).
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    /// A symbolic link or a special file is in the tree.
    #[error("unsupported file type: {0}")]
    Unsupported(PathBuf),
    /// The copy differs from its source.
    #[error("copy does not match its source: {0}")]
    Mismatch(PathBuf),
    /// A filesystem call failed.
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> MigrationError + '_ {
    move |e| MigrationError::Io(path.to_owned(), e)
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().map(std::ffi::OsStr::to_os_string).unwrap_or_default();
    name.push(".");
    name.push(suffix);
    path.with_file_name(name)
}

/// The staging folder used for `current`.
pub fn staging_dir(current: &Path) -> PathBuf {
    sibling(current, STAGING_SUFFIX)
}

/// Copies `legacy` to `current` under the rules of the module comment.
///
/// # Errors
///
/// [`MigrationError`]; the old root is then still the default and intact.
pub fn migrate(legacy: &Path, current: &Path) -> Result<Outcome, MigrationError> {
    if std::fs::symlink_metadata(current).is_ok() {
        return Ok(Outcome::AlreadyMigrated);
    }
    match std::fs::symlink_metadata(legacy) {
        Ok(meta) if meta.file_type().is_dir() => {}
        Ok(_) => return Err(MigrationError::Unsupported(legacy.to_owned())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Outcome::NothingToMove),
        Err(e) => return Err(io(legacy)(e)),
    }
    // Held until the end of the copy: no daemon starts on the old root.
    let lock_path = legacy.join(DAEMON_LOCK);
    let _lock = match File::options().read(true).write(true).open(&lock_path) {
        Ok(file) => match file.try_lock() {
            Ok(()) => Some(file),
            Err(std::fs::TryLockError::WouldBlock) => return Ok(Outcome::InUse),
            Err(std::fs::TryLockError::Error(e)) => return Err(io(&lock_path)(e)),
        },
        // A root no daemon of this generation ever ran on has no lock file.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(io(&lock_path)(e)),
    };
    if let Some(parent) = current.parent() {
        platform_dirs::ensure_private_dir(parent)
            .map_err(|e| MigrationError::Io(parent.to_owned(), std::io::Error::other(e)))?;
    }
    let staging = staging_dir(current);
    if std::fs::symlink_metadata(&staging).is_ok() {
        // The desktop's own partial copy from an earlier run.
        std::fs::remove_dir_all(&staging).map_err(io(&staging))?;
    }
    let result = copy_tree(legacy, &staging).and_then(|()| verify_tree(legacy, &staging)).and_then(
        |totals| {
            sync_dir(&staging);
            std::fs::rename(&staging, current).map_err(io(current))?;
            if let Some(parent) = current.parent() {
                sync_dir(parent);
            }
            Ok(totals)
        },
    );
    match result {
        Ok((files, bytes)) => {
            // Informational only; the switch has happened.
            let _ =
                std::fs::write(sibling(current, RECORD_SUFFIX), format!("{}\n", legacy.display()));
            Ok(Outcome::Migrated { files, bytes })
        }
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging);
            Err(error)
        }
    }
}

fn sync_dir(dir: &Path) {
    // Best effort: not every platform syncs a directory handle.
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), MigrationError> {
    let meta = std::fs::symlink_metadata(from).map_err(io(from))?;
    std::fs::create_dir(to).map_err(io(to))?;
    for entry in std::fs::read_dir(from).map_err(io(from))? {
        let entry = entry.map_err(io(from))?;
        let source = entry.path();
        let target = to.join(entry.file_name());
        let kind = entry.file_type().map_err(io(&source))?;
        if kind.is_dir() {
            copy_tree(&source, &target)?;
        } else if kind.is_file() {
            std::fs::copy(&source, &target).map_err(io(&source))?;
            File::open(&target).and_then(|f| f.sync_all()).map_err(io(&target))?;
        } else {
            return Err(MigrationError::Unsupported(source));
        }
    }
    // Last, so a read-only source folder does not stop its own copy.
    std::fs::set_permissions(to, meta.permissions()).map_err(io(to))?;
    sync_dir(to);
    Ok(())
}

/// Compares the trees; returns the regular files and bytes compared.
fn verify_tree(from: &Path, to: &Path) -> Result<(u64, u64), MigrationError> {
    let (mut files, mut bytes) = (0, 0);
    let mut sources = 0usize;
    for entry in std::fs::read_dir(from).map_err(io(from))? {
        let entry = entry.map_err(io(from))?;
        sources += 1;
        let source = entry.path();
        let target = to.join(entry.file_name());
        let kind = entry.file_type().map_err(io(&source))?;
        let copied = std::fs::symlink_metadata(&target)
            .map_err(|_| MigrationError::Mismatch(target.clone()))?;
        if kind.is_dir() && copied.file_type().is_dir() {
            let (f, b) = verify_tree(&source, &target)?;
            files += f;
            bytes += b;
        } else if kind.is_file() && copied.file_type().is_file() {
            bytes += same_contents(&source, &target)?;
            files += 1;
        } else {
            return Err(MigrationError::Mismatch(target));
        }
    }
    let copies = std::fs::read_dir(to).map_err(io(to))?.count();
    if copies != sources {
        return Err(MigrationError::Mismatch(to.to_owned()));
    }
    Ok((files, bytes))
}

/// Compares two files byte for byte; returns their length.
fn same_contents(a: &Path, b: &Path) -> Result<u64, MigrationError> {
    let mut left = File::open(a).map_err(io(a))?;
    let mut right = File::open(b).map_err(io(b))?;
    let (mut x, mut y) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
    let mut total = 0u64;
    loop {
        let n = read_full(&mut left, &mut x).map_err(io(a))?;
        let m = read_full(&mut right, &mut y).map_err(io(b))?;
        if n != m || x[..n] != y[..m] {
            return Err(MigrationError::Mismatch(b.to_owned()));
        }
        if n == 0 {
            return Ok(total);
        }
        total += n as u64;
    }
}

fn read_full(file: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("rf-migrate-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch");
            Self(dir)
        }
        fn legacy(&self) -> PathBuf {
            self.0.join("config/ramflux-desktop/rfd-data")
        }
        fn current(&self) -> PathBuf {
            self.0.join("share/ramflux-desktop/rfd-data")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn populate(root: &Path) {
        std::fs::create_dir_all(root.join("accounts/a1/journal")).expect("dirs");
        std::fs::write(root.join(DAEMON_LOCK), "").expect("lock");
        std::fs::write(root.join("accounts/a1/manifest.json"), r#"{"id":"a1"}"#).expect("f");
        let big: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(root.join("accounts/a1/journal/0001.log"), big).expect("big");
        std::fs::write(root.join("accounts/empty.bin"), "").expect("empty");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let set = |p: PathBuf, m| {
                std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).expect("mode")
            };
            set(root.to_owned(), 0o700);
            set(root.join("accounts/a1/manifest.json"), 0o600);
        }
    }

    fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_owned()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    stack.push(path.clone());
                    out.push((path.strip_prefix(root).expect("rel").to_owned(), Vec::new()));
                } else {
                    let bytes = std::fs::read(&path).expect("file");
                    out.push((path.strip_prefix(root).expect("rel").to_owned(), bytes));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn a_populated_root_is_copied_verified_and_switched() {
        let s = Scratch::new("copy");
        populate(&s.legacy());
        let before = snapshot(&s.legacy());

        let outcome = migrate(&s.legacy(), &s.current()).expect("migrated");
        assert!(matches!(outcome, Outcome::Migrated { files: 4, bytes } if bytes > 200_000));
        assert_eq!(snapshot(&s.current()), before, "same tree and contents");
        assert_eq!(snapshot(&s.legacy()), before, "the old root is untouched");
        assert!(!staging_dir(&s.current()).exists(), "no staging folder left");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = |p: PathBuf| std::fs::metadata(p).expect("m").permissions().mode() & 0o777;
            assert_eq!(mode(s.current()), 0o700);
            assert_eq!(mode(s.current().join("accounts/a1/manifest.json")), 0o600);
            assert_eq!(mode(s.current().parent().expect("parent").to_owned()), 0o700);
        }

        // Idempotent: a second run changes nothing.
        std::fs::write(s.legacy().join("accounts/later.txt"), "after").expect("later");
        assert_eq!(migrate(&s.legacy(), &s.current()).expect("again"), Outcome::AlreadyMigrated);
        assert!(!s.current().join("accounts/later.txt").exists());
    }

    #[test]
    fn nothing_to_move_without_an_old_root() {
        let s = Scratch::new("none");
        assert_eq!(migrate(&s.legacy(), &s.current()).expect("ok"), Outcome::NothingToMove);
        assert!(!s.current().exists());
    }

    #[test]
    fn a_running_daemon_defers_the_copy() {
        let s = Scratch::new("busy");
        populate(&s.legacy());
        let daemon = File::options()
            .read(true)
            .write(true)
            .open(s.legacy().join(DAEMON_LOCK))
            .expect("lock file");
        daemon.lock().expect("daemon lock");
        assert_eq!(migrate(&s.legacy(), &s.current()).expect("ok"), Outcome::InUse);
        assert!(!s.current().exists() && !staging_dir(&s.current()).exists());
        drop(daemon);
        assert!(matches!(
            migrate(&s.legacy(), &s.current()).expect("ok"),
            Outcome::Migrated { .. }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn an_unsupported_entry_abandons_the_copy() {
        let s = Scratch::new("link");
        populate(&s.legacy());
        std::os::unix::fs::symlink("/etc/hosts", s.legacy().join("accounts/link")).expect("ln");
        let before = snapshot(&s.legacy());
        assert!(matches!(
            migrate(&s.legacy(), &s.current()),
            Err(MigrationError::Unsupported(path)) if path.ends_with("accounts/link")
        ));
        assert!(!s.current().exists(), "no switch");
        assert!(!staging_dir(&s.current()).exists(), "partial copy removed");
        assert_eq!(snapshot(&s.legacy()), before, "the old root is untouched");
    }

    #[test]
    fn a_partial_copy_from_an_interrupted_run_is_replaced() {
        let s = Scratch::new("partial");
        populate(&s.legacy());
        let staging = staging_dir(&s.current());
        std::fs::create_dir_all(staging.join("accounts")).expect("staging");
        std::fs::write(staging.join("accounts/stale"), "stale").expect("stale");
        assert!(matches!(
            migrate(&s.legacy(), &s.current()).expect("ok"),
            Outcome::Migrated { .. }
        ));
        assert!(!s.current().join("accounts/stale").exists());
        assert_eq!(snapshot(&s.current()), snapshot(&s.legacy()));
    }

    #[test]
    fn verification_catches_a_difference() {
        let s = Scratch::new("verify");
        populate(&s.legacy());
        let copy = s.0.join("copy");
        copy_tree(&s.legacy(), &copy).expect("copy");
        assert!(verify_tree(&s.legacy(), &copy).is_ok());
        std::fs::write(copy.join("accounts/a1/manifest.json"), r#"{"id":"a2"}"#).expect("w");
        assert!(matches!(verify_tree(&s.legacy(), &copy), Err(MigrationError::Mismatch(_))));
        std::fs::write(copy.join("accounts/a1/manifest.json"), r#"{"id":"a1"}"#).expect("w");
        std::fs::write(copy.join("extra"), "x").expect("extra");
        assert!(matches!(verify_tree(&s.legacy(), &copy), Err(MigrationError::Mismatch(_))));
    }
}
