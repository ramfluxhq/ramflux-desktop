// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A local daemon the desktop started on the user's request, and owns.
//!
//! The desktop never starts a daemon by itself: [`DaemonProcess::spawn`]
//! is called only after the user pressed "Start local daemon". It runs
//! `rf --socket <socket> daemon start --data-root <data dir>` as a child
//! process, with its output appended to a log file.
//!
//! A daemon the desktop owns is stopped gracefully over the local bus
//! (`daemon.stop`, through the bridge); this handle is the bounded fallback
//! when that is refused or does not end the process in time: a terminate
//! signal first, then by force. Dropping the handle stops it that way.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::app_log;

/// How long a terminated daemon may take to exit before it is killed.
pub const STOP_GRACE: Duration = Duration::from_secs(3);

/// What to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonCommand {
    /// The `rf` binary.
    pub binary: PathBuf,
    /// The local-bus socket the daemon listens on.
    pub socket: PathBuf,
    /// The daemon's data root.
    pub data_dir: PathBuf,
    /// Where the daemon's output goes; `None` discards it.
    pub log_file: Option<PathBuf>,
}

impl DaemonCommand {
    /// The arguments after the binary.
    pub fn args(&self) -> Vec<std::ffi::OsString> {
        vec![
            "--socket".into(),
            self.socket.clone().into_os_string(),
            "daemon".into(),
            "start".into(),
            "--data-root".into(),
            self.data_dir.clone().into_os_string(),
        ]
    }
}

/// Why a daemon was not started.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LaunchError {
    /// No daemon binary is set and none is bundled.
    #[error("no daemon binary is set")]
    BinaryMissing,
    /// The set binary does not exist.
    #[error("the daemon binary does not exist: {0}")]
    BinaryNotFound(PathBuf),
    /// The set path is not an executable file.
    #[error("the daemon binary is not an executable file: {0}")]
    NotExecutable(PathBuf),
    /// No data directory could be determined.
    #[error("no data directory is set")]
    NoDataDir,
    /// The process could not be started.
    #[error("cannot start the daemon: {0}")]
    Spawn(String),
}

/// Checks that `binary` is an existing executable file.
///
/// # Errors
///
/// [`LaunchError::BinaryNotFound`] or [`LaunchError::NotExecutable`].
pub fn check_binary(binary: &Path) -> Result<(), LaunchError> {
    let meta =
        std::fs::metadata(binary).map_err(|_| LaunchError::BinaryNotFound(binary.to_owned()))?;
    if !meta.is_file() {
        return Err(LaunchError::NotExecutable(binary.to_owned()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if meta.permissions().mode() & 0o111 == 0 {
            return Err(LaunchError::NotExecutable(binary.to_owned()));
        }
    }
    Ok(())
}

fn output_file(path: Option<&Path>) -> Option<(File, File)> {
    let path = path?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok()?;
    }
    app_log::rotate(path, app_log::MAX_LOG_BYTES, app_log::KEEP_ROTATED);
    let out = File::options().create(true).append(true).open(path).ok()?;
    let err = out.try_clone().ok()?;
    Some((out, err))
}

/// A running daemon child process.
#[derive(Debug)]
pub struct DaemonProcess {
    child: Option<Child>,
    pid: u32,
}

impl DaemonProcess {
    /// Starts `command`.
    ///
    /// # Errors
    ///
    /// A missing or non-executable binary, or a failed spawn.
    pub fn spawn(command: &DaemonCommand) -> Result<Self, LaunchError> {
        check_binary(&command.binary)?;
        let (stdout, stderr) = match output_file(command.log_file.as_deref()) {
            Some((out, err)) => (Stdio::from(out), Stdio::from(err)),
            None => (Stdio::null(), Stdio::null()),
        };
        let child = Command::new(&command.binary)
            .args(command.args())
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .spawn()
            .map_err(|e| LaunchError::Spawn(e.to_string()))?;
        let pid = child.id();
        Ok(Self { child: Some(child), pid })
    }

    /// The process id.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// The exit code once the process has exited (`Some(None)` when it was
    /// ended by a signal); `None` while it runs.
    pub fn exit_code(&mut self) -> Option<Option<i32>> {
        let child = self.child.as_mut()?;
        match child.try_wait() {
            Ok(Some(status)) => Some(status.code()),
            Ok(None) => None,
            Err(_) => Some(None),
        }
    }

    /// Asks the process to exit (a terminate signal); returns at once.
    pub fn terminate(&mut self) {
        if self.child.is_some() && self.exit_code().is_none() {
            #[cfg(unix)]
            {
                let _ = Command::new("kill")
                    .args(["-TERM", &self.pid.to_string()])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            #[cfg(not(unix))]
            self.kill();
        }
    }

    /// Ends the process by force and reaps it.
    pub fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Waits up to `limit` for the process to exit; returns whether it
    /// did. Blocks the caller for at most `limit`.
    pub fn wait_exit(&mut self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            if self.child.is_none() || self.exit_code().is_some() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Terminates the process and waits up to `grace` for it to exit, then
    /// kills it. Blocks the caller for at most `grace`.
    pub fn stop(&mut self, grace: Duration) {
        self.terminate();
        self.wait_exit(grace);
        self.kill();
    }
}

impl Drop for DaemonProcess {
    fn drop(&mut self) {
        if self.child.is_some() {
            self.stop(STOP_GRACE);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rf-proc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        dir
    }

    /// A stand-in daemon: records its arguments, then waits.
    fn fake_rf(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join("rf");
        let script = format!(
            "#!/bin/sh\necho \"$@\" > '{}'\necho started\nexec sleep 30\n",
            dir.join("args").display()
        );
        std::fs::write(&path, script).expect("script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("mode");
        path
    }

    #[test]
    fn a_started_daemon_gets_the_socket_and_data_root_and_stops() {
        let dir = dir("run");
        let command = DaemonCommand {
            binary: fake_rf(&dir),
            socket: dir.join("rfd.sock"),
            data_dir: dir.join("data"),
            log_file: Some(dir.join("logs/rfd.log")),
        };
        let mut process = DaemonProcess::spawn(&command).expect("spawn");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !dir.join("args").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let args = std::fs::read_to_string(dir.join("args")).expect("args");
        assert_eq!(
            args.trim(),
            format!(
                "--socket {} daemon start --data-root {}",
                dir.join("rfd.sock").display(),
                dir.join("data").display()
            )
        );
        assert_eq!(process.exit_code(), None, "still running");
        process.stop(Duration::from_secs(3));
        assert!(process.child.is_none(), "reaped");
        let log = std::fs::read_to_string(dir.join("logs/rfd.log")).expect("log");
        assert!(log.contains("started"), "output goes to the log");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_and_non_executable_binaries_are_refused() {
        let dir = dir("refuse");
        let command = |binary: PathBuf| DaemonCommand {
            binary,
            socket: dir.join("s"),
            data_dir: dir.join("d"),
            log_file: None,
        };
        let missing = dir.join("nope");
        assert_eq!(
            DaemonProcess::spawn(&command(missing.clone())).err(),
            Some(LaunchError::BinaryNotFound(missing))
        );
        let plain = dir.join("plain");
        std::fs::write(&plain, "not a program").expect("file");
        assert_eq!(
            DaemonProcess::spawn(&command(plain.clone())).err(),
            Some(LaunchError::NotExecutable(plain))
        );
        assert_eq!(
            DaemonProcess::spawn(&command(dir.clone())).err(),
            Some(LaunchError::NotExecutable(dir.clone())),
            "a directory is not a binary"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
