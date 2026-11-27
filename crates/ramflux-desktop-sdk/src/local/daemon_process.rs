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
    /// The PRX state directory the daemon adopts its runtime from (after an
    /// install or a restart of the runtime). `None` starts without PRX.
    pub prx_state_dir: Option<PathBuf>,
}

impl DaemonCommand {
    /// The arguments after the binary.
    pub fn args(&self) -> Vec<std::ffi::OsString> {
        let mut args: Vec<std::ffi::OsString> = vec![
            "--socket".into(),
            self.socket.clone().into_os_string(),
            "daemon".into(),
            "start".into(),
            "--data-root".into(),
            self.data_dir.clone().into_os_string(),
        ];
        if let Some(dir) = &self.prx_state_dir {
            args.push("--prx-state-dir".into());
            args.push(dir.clone().into_os_string());
        }
        args
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

/// Builds the child command separately from opening its output files.
fn child_command(command: &DaemonCommand) -> Command {
    let mut child = Command::new(&command.binary);
    child.args(command.args()).stdin(Stdio::null());
    set_child_log(&mut child, std::env::var_os("RUST_LOG"));
    set_child_username_center(&mut child);
    child
}

/// Hands the daemon the username centre this computer uses (the bundled
/// pin, or the environment override), so its username answers are checked
/// against the same authority the desktop authorizes with.
fn set_child_username_center(child: &mut Command) {
    if let Some(pin) = crate::username_center::current() {
        child
            .env("RAMFLUX_USERNAME_CENTER_URL", pin.url)
            .env("RAMFLUX_USERNAME_CENTER_ID", pin.id)
            .env("RAMFLUX_USERNAME_CENTER_PUBLIC_KEY", pin.public_key)
            .env("RAMFLUX_USERNAME_TRUST_GENERATION", pin.generation.to_string());
    }
}

/// Keep the parent filter, including an explicitly empty filter.
fn set_child_log(child: &mut Command, inherited: Option<std::ffi::OsString>) {
    child.env("RUST_LOG", inherited.unwrap_or_else(|| "warn,ramflux_sdk=info".into()));
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
        let child = child_command(command)
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

    // A started daemon's arguments, output and stop are tested in
    // `tests/daemon_process.rs`, whose binary stands in for the daemon.

    #[test]
    fn daemon_spawn_sets_default_rust_log_unless_inherited() {
        // Exercise the same child configuration as spawn without mutating the
        // process environment (other tests start bridge threads concurrently).
        for inherited in [None, Some("trace"), Some("")] {
            let mut child = Command::new("sh");
            child.args(["-c", "printf '%s' \"$RUST_LOG\""]);
            set_child_log(&mut child, inherited.map(Into::into));
            let output = child.output().expect("child");
            assert!(output.status.success());
            assert_eq!(output.stdout, inherited.unwrap_or("warn,ramflux_sdk=info").as_bytes());
        }
        let command = DaemonCommand {
            binary: "rf".into(),
            socket: "s".into(),
            data_dir: "d".into(),
            log_file: None,
            prx_state_dir: None,
        };
        let child = child_command(&command);
        let filter = child.get_envs().find(|(key, _)| *key == "RUST_LOG").expect("child filter").1;
        assert_eq!(
            filter,
            Some(
                std::env::var_os("RUST_LOG")
                    .unwrap_or_else(|| "warn,ramflux_sdk=info".into())
                    .as_os_str()
            )
        );
    }

    #[test]
    fn missing_and_non_executable_binaries_are_refused() {
        let dir = dir("refuse");
        let command = |binary: PathBuf| DaemonCommand {
            binary,
            socket: dir.join("s"),
            data_dir: dir.join("d"),
            log_file: None,
            prx_state_dir: None,
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

#[cfg(test)]
mod prx_state_dir_tests {
    use super::*;

    #[test]
    fn the_daemon_adopts_the_runtime_from_its_state_directory_when_given() {
        let mut command = DaemonCommand {
            binary: "rf".into(),
            socket: "s".into(),
            data_dir: "d".into(),
            log_file: None,
            prx_state_dir: None,
        };
        assert!(!command.args().iter().any(|arg| arg == "--prx-state-dir"));
        command.prx_state_dir = Some("/state/prx".into());
        let args = command.args();
        let at = args.iter().position(|arg| arg == "--prx-state-dir").expect("the flag");
        assert_eq!(args[at + 1], "/state/prx");
        assert_eq!(args[4], "--data-root", "the flag follows the data root");
    }
}
