// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The operating system's per-user directories, in one place.
//!
//! Every path the desktop derives from the environment goes through
//! [`resolve`]: [`super::paths`] lays the desktop's own files out on top of
//! it. Resolution is a pure function of a [`Platform`] and an [`Env`]
//! snapshot, so each platform's rules are tested on any host.
//!
//! | Kind | Linux and other Unix (XDG) | macOS | Windows (not used yet) |
//! |---|---|---|---|
//! | config | `$XDG_CONFIG_HOME/ramflux-desktop` | `~/Library/Application Support/Ramflux Desktop` | `%APPDATA%\Ramflux Desktop` |
//! | data | `$XDG_DATA_HOME/ramflux-desktop` | as config | `%LOCALAPPDATA%\Ramflux Desktop\data` |
//! | state | `$XDG_STATE_HOME/ramflux-desktop` | as config | `%LOCALAPPDATA%\Ramflux Desktop` |
//! | logs | `<state>/logs` | `~/Library/Logs/Ramflux Desktop` | `<state>\logs` |
//! | cache | `$XDG_CACHE_HOME/ramflux-desktop` | `~/Library/Caches/Ramflux Desktop` | `%LOCALAPPDATA%\Ramflux Desktop\cache` |
//! | runtime | `$XDG_RUNTIME_DIR` | none | none |
//!
//! XDG variables that are unset, empty or relative are ignored, as the
//! Base Directory specification requires, and the documented defaults under
//! `$HOME` apply (`~/.config`, `~/.local/share`, `~/.local/state`,
//! `~/.cache`). There is no default for the runtime directory: without
//! `$XDG_RUNTIME_DIR` the caller picks a private fallback (see
//! [`super::paths::default_socket`]). The Windows column only names the
//! conventional locations so later Windows work has one place to change; no
//! Windows behaviour is claimed or tested on Windows.
//!
//! Nothing here creates a directory except [`ensure_private_dir`].

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::identity::{NATIVE_DIR_NAME, XDG_DIR_NAME};

/// Which convention applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// The XDG Base Directory specification (Linux, the BSDs).
    Xdg,
    /// macOS per-user Library folders.
    MacOs,
    /// Windows known folders (environment variables only for now).
    Windows,
}

impl Platform {
    /// The platform this build runs on.
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Xdg
        }
    }
}

/// The environment variables resolution reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    /// `HOME` (`USERPROFILE` on Windows).
    pub home: Option<OsString>,
    /// `XDG_CONFIG_HOME`.
    pub config_home: Option<OsString>,
    /// `XDG_DATA_HOME`.
    pub data_home: Option<OsString>,
    /// `XDG_STATE_HOME`.
    pub state_home: Option<OsString>,
    /// `XDG_CACHE_HOME`.
    pub cache_home: Option<OsString>,
    /// `XDG_RUNTIME_DIR`.
    pub runtime_dir: Option<OsString>,
    /// `APPDATA` (Windows).
    pub app_data: Option<OsString>,
    /// `LOCALAPPDATA` (Windows).
    pub local_app_data: Option<OsString>,
}

impl Env {
    /// A snapshot of this process's environment.
    pub fn from_process() -> Self {
        let var = std::env::var_os;
        Self {
            home: var("HOME").or_else(|| if cfg!(windows) { var("USERPROFILE") } else { None }),
            config_home: var("XDG_CONFIG_HOME"),
            data_home: var("XDG_DATA_HOME"),
            state_home: var("XDG_STATE_HOME"),
            cache_home: var("XDG_CACHE_HOME"),
            runtime_dir: var("XDG_RUNTIME_DIR"),
            app_data: var("APPDATA"),
            local_app_data: var("LOCALAPPDATA"),
        }
    }
}

/// The desktop's per-user directories on one platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    /// The user's home folder.
    pub home: PathBuf,
    /// Settings.
    pub config: PathBuf,
    /// Data that must survive (a started daemon's data root lives here).
    pub data: PathBuf,
    /// State that may be lost without harm (logs).
    pub state: PathBuf,
    /// Logs.
    pub logs: PathBuf,
    /// Re-creatable files.
    pub cache: PathBuf,
    /// The session's private runtime directory (sockets), when the
    /// platform has one; not suffixed with the application folder.
    pub runtime: Option<PathBuf>,
}

/// An absolute path from a variable; empty or relative values are ignored.
fn absolute(value: Option<&OsString>) -> Option<PathBuf> {
    value.filter(|v| !v.is_empty()).map(PathBuf::from).filter(|p| p.is_absolute())
}

/// The user's home folder; `None` when unknown or relative.
pub fn home(env: &Env) -> Option<PathBuf> {
    absolute(env.home.as_ref())
}

/// The directories of `platform` under `env`; `None` without a home folder
/// (and, on Windows, without the known-folder variables).
pub fn resolve(platform: Platform, env: &Env) -> Option<Dirs> {
    let home = home(env)?;
    Some(match platform {
        Platform::Xdg => {
            let base = |value: Option<&OsString>, default: &str| {
                absolute(value).unwrap_or_else(|| home.join(default)).join(XDG_DIR_NAME)
            };
            let state = base(env.state_home.as_ref(), ".local/state");
            Dirs {
                config: base(env.config_home.as_ref(), ".config"),
                data: base(env.data_home.as_ref(), ".local/share"),
                logs: state.join("logs"),
                state,
                cache: base(env.cache_home.as_ref(), ".cache"),
                runtime: absolute(env.runtime_dir.as_ref()),
                home,
            }
        }
        Platform::MacOs => {
            let support = home.join("Library/Application Support").join(NATIVE_DIR_NAME);
            Dirs {
                config: support.clone(),
                data: support.clone(),
                state: support,
                logs: home.join("Library/Logs").join(NATIVE_DIR_NAME),
                cache: home.join("Library/Caches").join(NATIVE_DIR_NAME),
                runtime: None,
                home,
            }
        }
        Platform::Windows => {
            let roaming = absolute(env.app_data.as_ref())?.join(NATIVE_DIR_NAME);
            let local = absolute(env.local_app_data.as_ref())?.join(NATIVE_DIR_NAME);
            Dirs {
                config: roaming,
                data: local.join("data"),
                logs: local.join("logs"),
                cache: local.join("cache"),
                state: local,
                runtime: None,
                home,
            }
        }
    })
}

/// The directories of this platform and process.
pub fn current() -> Option<Dirs> {
    resolve(Platform::current(), &Env::from_process())
}

/// Why a private directory could not be prepared.
#[derive(Debug, thiserror::Error)]
pub enum PrivateDirError {
    /// The path exists but is a symbolic link or not a directory.
    #[error("not a plain directory: {0}")]
    NotADirectory(PathBuf),
    /// The directory belongs to another user than its parent.
    #[error("owned by another user: {0}")]
    ForeignOwner(PathBuf),
    /// A filesystem call failed.
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
}

/// Creates `dir` (and missing parents) so that only its owner can enter it
/// (mode 0700 on Unix), or tightens an existing one.
///
/// An existing `dir` must be a real directory (not a symbolic link) owned by
/// the same user as its parent, which for the directories this is used on
/// (under `$XDG_RUNTIME_DIR` or the user's state folder) is the user;
/// otherwise it is refused and left alone. On other platforms the directory
/// is only created.
///
/// # Errors
///
/// [`PrivateDirError`] as above.
pub fn ensure_private_dir(dir: &Path) -> Result<(), PrivateDirError> {
    let io = |e| PrivateDirError::Io(dir.to_owned(), e);
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, PermissionsExt as _};
        match std::fs::DirBuilder::new().mode(0o700).create(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(io(e)),
        }
        let meta = std::fs::symlink_metadata(dir).map_err(io)?;
        if !meta.file_type().is_dir() {
            return Err(PrivateDirError::NotADirectory(dir.to_owned()));
        }
        // The folder is this process user's, not merely the same as its
        // parent's (the parent can be anyone's).
        if meta.uid() != rustix::process::getuid().as_raw() {
            return Err(PrivateDirError::ForeignOwner(dir.to_owned()));
        }
        if meta.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(io)?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir).map_err(io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Env {
        let mut env = Env::default();
        for (key, value) in pairs {
            let value = Some(OsString::from(value));
            match *key {
                "HOME" => env.home = value,
                "XDG_CONFIG_HOME" => env.config_home = value,
                "XDG_DATA_HOME" => env.data_home = value,
                "XDG_STATE_HOME" => env.state_home = value,
                "XDG_CACHE_HOME" => env.cache_home = value,
                "XDG_RUNTIME_DIR" => env.runtime_dir = value,
                "APPDATA" => env.app_data = value,
                "LOCALAPPDATA" => env.local_app_data = value,
                other => panic!("unknown variable {other}"),
            }
        }
        env
    }

    #[test]
    fn xdg_defaults_live_under_home() {
        let dirs = resolve(Platform::Xdg, &env(&[("HOME", "/home/u")])).expect("dirs");
        assert_eq!(dirs.config, PathBuf::from("/home/u/.config/ramflux-desktop"));
        assert_eq!(dirs.data, PathBuf::from("/home/u/.local/share/ramflux-desktop"));
        assert_eq!(dirs.state, PathBuf::from("/home/u/.local/state/ramflux-desktop"));
        assert_eq!(dirs.logs, PathBuf::from("/home/u/.local/state/ramflux-desktop/logs"));
        assert_eq!(dirs.cache, PathBuf::from("/home/u/.cache/ramflux-desktop"));
        assert_eq!(dirs.runtime, None, "no default runtime directory");
    }

    #[test]
    fn xdg_variables_win_when_absolute() {
        let dirs = resolve(
            Platform::Xdg,
            &env(&[
                ("HOME", "/home/u"),
                ("XDG_CONFIG_HOME", "/c"),
                ("XDG_DATA_HOME", "/d"),
                ("XDG_STATE_HOME", "/s"),
                ("XDG_CACHE_HOME", "/k"),
                ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ]),
        )
        .expect("dirs");
        assert_eq!(dirs.config, PathBuf::from("/c/ramflux-desktop"));
        assert_eq!(dirs.data, PathBuf::from("/d/ramflux-desktop"));
        assert_eq!(dirs.state, PathBuf::from("/s/ramflux-desktop"));
        assert_eq!(dirs.logs, PathBuf::from("/s/ramflux-desktop/logs"));
        assert_eq!(dirs.cache, PathBuf::from("/k/ramflux-desktop"));
        assert_eq!(dirs.runtime, Some(PathBuf::from("/run/user/1000")));
    }

    #[test]
    fn empty_and_relative_xdg_values_are_ignored() {
        let dirs = resolve(
            Platform::Xdg,
            &env(&[
                ("HOME", "/home/u"),
                ("XDG_CONFIG_HOME", ""),
                ("XDG_DATA_HOME", "relative/data"),
                ("XDG_RUNTIME_DIR", "run"),
            ]),
        )
        .expect("dirs");
        assert_eq!(dirs.config, PathBuf::from("/home/u/.config/ramflux-desktop"));
        assert_eq!(dirs.data, PathBuf::from("/home/u/.local/share/ramflux-desktop"));
        assert_eq!(dirs.runtime, None);
        assert_eq!(resolve(Platform::Xdg, &env(&[("HOME", "home")])), None, "relative home");
        assert_eq!(resolve(Platform::Xdg, &Env::default()), None, "no home");
    }

    #[test]
    fn macos_paths_are_the_library_folders() {
        let dirs = resolve(
            Platform::MacOs,
            &env(&[("HOME", "/Users/u"), ("XDG_CONFIG_HOME", "/ignored")]),
        )
        .expect("dirs");
        let support = PathBuf::from("/Users/u/Library/Application Support/Ramflux Desktop");
        assert_eq!(dirs.config, support);
        assert_eq!(dirs.data, support);
        assert_eq!(dirs.state, support);
        assert_eq!(dirs.logs, PathBuf::from("/Users/u/Library/Logs/Ramflux Desktop"));
        assert_eq!(dirs.cache, PathBuf::from("/Users/u/Library/Caches/Ramflux Desktop"));
        assert_eq!(dirs.runtime, None);
    }

    #[test]
    fn windows_names_the_known_folders() {
        let dirs = resolve(
            Platform::Windows,
            &env(&[("HOME", "/u"), ("APPDATA", "/u/roaming"), ("LOCALAPPDATA", "/u/local")]),
        )
        .expect("dirs");
        assert_eq!(dirs.config, PathBuf::from("/u/roaming/Ramflux Desktop"));
        assert_eq!(dirs.state, PathBuf::from("/u/local/Ramflux Desktop"));
        assert_eq!(dirs.data, PathBuf::from("/u/local/Ramflux Desktop/data"));
        assert_eq!(dirs.logs, PathBuf::from("/u/local/Ramflux Desktop/logs"));
        assert_eq!(dirs.cache, PathBuf::from("/u/local/Ramflux Desktop/cache"));
        assert_eq!(resolve(Platform::Windows, &env(&[("HOME", "/u")])), None);
    }

    #[cfg(unix)]
    #[test]
    fn private_dirs_are_created_and_tightened() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = std::env::temp_dir().join(format!("rf-private-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("run/ramflux");
        ensure_private_dir(&dir).expect("created");
        let mode = |p: &Path| std::fs::metadata(p).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("loosen");
        ensure_private_dir(&dir).expect("again");
        assert_eq!(mode(&dir), 0o700, "an existing directory is tightened");

        let file = root.join("file");
        std::fs::write(&file, "x").expect("file");
        assert!(matches!(ensure_private_dir(&file), Err(PrivateDirError::NotADirectory(_))));
        let link = root.join("link");
        std::os::unix::fs::symlink(&dir, &link).expect("link");
        assert!(
            matches!(ensure_private_dir(&link), Err(PrivateDirError::NotADirectory(_))),
            "a symbolic link is refused, not followed"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
