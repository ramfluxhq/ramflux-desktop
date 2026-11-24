// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Where the desktop keeps its files on this machine, laid out on the
//! platform directories of [`super::platform_dirs`].
//!
//! | What | macOS | Linux (XDG) |
//! |---|---|---|
//! | settings | `~/Library/Application Support/Ramflux Desktop` | `$XDG_CONFIG_HOME/ramflux-desktop` |
//! | logs | `~/Library/Logs/Ramflux Desktop` | `$XDG_STATE_HOME/ramflux-desktop/logs` |
//! | daemon data (default) | `<settings>/rfd-data` | `$XDG_DATA_HOME/ramflux-desktop/rfd-data` |
//! | daemon socket (default) | `/tmp/ramflux/rfd.sock` | `$XDG_RUNTIME_DIR/ramflux/rfd.sock` |
//!
//! The default socket on Linux is in the session's runtime directory, never
//! in `/tmp` while one exists. Two exceptions keep an existing setup
//! working: a daemon already listening at the reference CLI's socket
//! (`/tmp/ramflux/rfd.sock`, the `rf` default) is attached to when nothing
//! listens in the runtime directory, and a runtime path too long for a Unix
//! socket falls back to that socket. Without `$XDG_RUNTIME_DIR` the socket
//! goes to a private folder in the state directory.
//!
//! `RAMFLUX_DESKTOP_STATE_DIR` and `RAMFLUX_DESKTOP_LOG_DIR` override the
//! settings and log directories (tests, side-by-side installs); with the
//! first set, daemon data stays under it as before. Nothing here is created;
//! the writers create what they need.

use std::path::{Path, PathBuf};

use super::platform_dirs::{self, Dirs, Platform};
use crate::config::DEFAULT_SOCKET;

/// Overrides the state directory.
pub const STATE_DIR_ENV: &str = "RAMFLUX_DESKTOP_STATE_DIR";

/// Overrides the log directory.
pub const LOG_DIR_ENV: &str = "RAMFLUX_DESKTOP_LOG_DIR";

/// File name of the desktop settings file in the state directory.
pub const SETTINGS_FILE: &str = "desktop-settings.json";

/// Folder name of a started daemon's data root.
pub const DATA_DIR_NAME: &str = "rfd-data";

/// Folder in the runtime directory holding the local-bus socket (shared
/// with the reference CLI's layout: `ramflux/rfd.sock`).
pub const RUNTIME_SUBDIR: &str = "ramflux";

/// File name of the local-bus socket.
pub const SOCKET_NAME: &str = "rfd.sock";

/// The longest socket path used, in bytes. A Unix socket address holds 108
/// bytes on Linux and 104 on macOS, including the terminating zero.
pub const MAX_SOCKET_PATH: usize = 100;

/// File name of a daemon binary shipped inside the application bundle,
/// next to the desktop executable.
pub const BUNDLED_DAEMON: &str = if cfg!(windows) { "rf.exe" } else { "rf" };

fn dirs() -> Option<Dirs> {
    platform_dirs::current()
}

fn state_override() -> Option<PathBuf> {
    std::env::var_os(STATE_DIR_ENV).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The user's home folder, if known.
pub fn home_dir() -> Option<PathBuf> {
    platform_dirs::home(&platform_dirs::Env::from_process())
}

/// The directory of the desktop's local settings files.
pub fn state_dir() -> Option<PathBuf> {
    state_override().or_else(|| dirs().map(|d| d.config))
}

/// The desktop settings file.
pub fn settings_file() -> Option<PathBuf> {
    state_dir().map(|dir| dir.join(SETTINGS_FILE))
}

/// The directory of the application log (and the log of a daemon the
/// desktop started).
pub fn log_dir() -> Option<PathBuf> {
    std::env::var_os(LOG_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs().map(|d| d.logs))
}

/// Where a daemon the desktop starts keeps its data when no other folder
/// is set; never in a temporary folder. Under the state override it is
/// `<state>/rfd-data`.
pub fn default_data_dir() -> Option<PathBuf> {
    if let Some(state) = state_override() {
        return Some(state.join(DATA_DIR_NAME));
    }
    dirs().map(|d| d.data.join(DATA_DIR_NAME))
}

/// The default local-bus socket and, when the desktop is responsible for
/// it, the private folder it must sit in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketChoice {
    /// The socket path.
    pub path: PathBuf,
    /// The folder the desktop prepares (mode 0700) before it starts a
    /// daemon on [`Self::path`]; `None` for the reference CLI's socket,
    /// whose folder the daemon creates.
    pub private_dir: Option<PathBuf>,
}

impl SocketChoice {
    fn legacy() -> Self {
        Self { path: PathBuf::from(DEFAULT_SOCKET), private_dir: None }
    }
}

/// The default socket on `platform` for `dirs` (see the module comment);
/// `exists` tells whether a socket path is present.
pub fn choose_socket(
    platform: Platform,
    dirs: Option<&Dirs>,
    exists: impl Fn(&Path) -> bool,
) -> SocketChoice {
    let (Platform::Xdg, Some(dirs)) = (platform, dirs) else {
        return SocketChoice::legacy();
    };
    let folder = dirs
        .runtime
        .as_ref()
        .map_or_else(|| dirs.state.join("run"), |runtime| runtime.join(RUNTIME_SUBDIR));
    let path = folder.join(SOCKET_NAME);
    if path.as_os_str().len() > MAX_SOCKET_PATH {
        return SocketChoice::legacy();
    }
    if !exists(&path) && exists(Path::new(DEFAULT_SOCKET)) {
        return SocketChoice::legacy();
    }
    SocketChoice { path, private_dir: Some(folder) }
}

/// The default socket of this process. Under the state override it stays
/// the reference CLI's socket.
pub fn default_socket_choice() -> SocketChoice {
    if state_override().is_some() {
        return SocketChoice::legacy();
    }
    choose_socket(Platform::current(), dirs().as_ref(), Path::exists)
}

/// The default socket path of this process.
pub fn default_socket() -> PathBuf {
    default_socket_choice().path
}

/// A daemon binary shipped next to the desktop executable (inside a
/// packaged application, or `/usr/bin/rf` beside `/usr/bin/ramflux-desktop`
/// when a package provides it), if there is one.
pub fn bundled_daemon() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let candidate = exe.parent()?.join(BUNDLED_DAEMON);
    candidate.is_file().then_some(candidate)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;
    use crate::local::platform_dirs::{Env, resolve};

    fn linux(runtime: Option<&str>) -> Dirs {
        let env = Env {
            home: Some(OsString::from("/home/u")),
            runtime_dir: runtime.map(OsString::from),
            ..Env::default()
        };
        resolve(Platform::Xdg, &env).expect("dirs")
    }

    #[test]
    fn the_settings_file_lives_in_the_state_dir() {
        if let (Some(state), Some(file)) = (state_dir(), settings_file()) {
            assert_eq!(file.parent(), Some(state.as_path()));
        }
    }

    #[test]
    fn linux_socket_lives_in_the_runtime_dir() {
        let dirs = linux(Some("/run/user/1000"));
        let choice = choose_socket(Platform::Xdg, Some(&dirs), |_| false);
        assert_eq!(choice.path, PathBuf::from("/run/user/1000/ramflux/rfd.sock"));
        assert_eq!(choice.private_dir, Some(PathBuf::from("/run/user/1000/ramflux")));
        assert!(!choice.path.starts_with("/tmp"));
    }

    #[test]
    fn a_daemon_on_the_cli_socket_is_attached_to() {
        let dirs = linux(Some("/run/user/1000"));
        let legacy = |p: &Path| p == Path::new(DEFAULT_SOCKET);
        assert_eq!(choose_socket(Platform::Xdg, Some(&dirs), legacy), SocketChoice::legacy());
        // Both present: the runtime socket wins.
        let both = choose_socket(Platform::Xdg, Some(&dirs), |_| true);
        assert_eq!(both.path, PathBuf::from("/run/user/1000/ramflux/rfd.sock"));
    }

    #[test]
    fn without_a_runtime_dir_the_socket_is_private_under_state() {
        let choice = choose_socket(Platform::Xdg, Some(&linux(None)), |_| false);
        assert_eq!(choice.path, PathBuf::from("/home/u/.local/state/ramflux-desktop/run/rfd.sock"));
        assert_eq!(
            choice.private_dir,
            Some(PathBuf::from("/home/u/.local/state/ramflux-desktop/run"))
        );
    }

    #[test]
    fn a_socket_path_too_long_falls_back_to_the_cli_socket() {
        let long = format!("/run/{}", "x".repeat(MAX_SOCKET_PATH));
        let choice = choose_socket(Platform::Xdg, Some(&linux(Some(&long))), |_| false);
        assert_eq!(choice, SocketChoice::legacy());
    }

    #[test]
    fn macos_and_windows_keep_the_cli_socket() {
        let dirs = linux(Some("/run/user/1000"));
        assert_eq!(choose_socket(Platform::MacOs, Some(&dirs), |_| false), SocketChoice::legacy());
        assert_eq!(choose_socket(Platform::Windows, None, |_| false), SocketChoice::legacy());
        assert_eq!(choose_socket(Platform::Xdg, None, |_| false), SocketChoice::legacy());
    }
}
