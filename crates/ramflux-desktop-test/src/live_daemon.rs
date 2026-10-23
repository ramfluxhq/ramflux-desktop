// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A real `rf daemon` for live tests, built from the product revision this
//! workspace pins (behind the `live-daemon` feature).
//!
//! Live tests are `#[ignore]`d and also need the switch
//! `RAMFLUX_LIVE_DAEMON=1`; `RAMFLUX_RF_BIN` names the `rf` binary (else `rf`
//! on `PATH`). When the binary was installed with `cargo install`, its install
//! record must name the pinned revision (read from `Cargo.lock`), so a test
//! never runs against another product build by accident.
//!
//! Each daemon gets a short directory `/tmp/rf-f3-<pid>-<tag>` (Unix socket
//! paths are short) holding its socket, data root and log. Dropping the
//! guard stops the daemon and removes the directory, also when a test
//! panics.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// The switch that enables live tests.
pub const ENABLE_ENV: &str = "RAMFLUX_LIVE_DAEMON";

/// The `rf` binary to run.
pub const RF_BIN_ENV: &str = "RAMFLUX_RF_BIN";

/// How long a daemon may take to answer after starting.
const READY_TIMEOUT: Duration = Duration::from_secs(20);

/// Whether live tests are switched on.
pub fn enabled() -> bool {
    std::env::var(ENABLE_ENV).is_ok_and(|value| value == "1")
}

/// The product revision the workspace pins (the `ramflux-sdk` git source
/// in `Cargo.lock`).
pub fn pinned_product_rev() -> Option<String> {
    let lock = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock");
    let text = fs::read_to_string(lock).ok()?;
    let mut in_sdk = false;
    for line in text.lines() {
        if line.starts_with("name = ") {
            in_sdk = line == "name = \"ramflux-sdk\"";
        } else if in_sdk && let Some(source) = line.strip_prefix("source = \"git+") {
            return source.split_once("rev=").map(|(_, rest)| {
                rest.split(['#', '"', '&']).next().unwrap_or_default().to_owned()
            });
        }
    }
    None
}

/// The `rf` binary, checked against the pinned revision when its install
/// record is available.
///
/// # Errors
///
/// Returns why the binary cannot be used: an install record that names
/// another revision.
pub fn rf_binary() -> Result<PathBuf, String> {
    let bin = std::env::var_os(RF_BIN_ENV).map_or_else(|| PathBuf::from("rf"), PathBuf::from);
    let record = bin.parent().and_then(Path::parent).map(|root| root.join(".crates2.json"));
    if let (Some(record), Some(rev)) = (record, pinned_product_rev())
        && let Ok(text) = fs::read_to_string(&record)
        && !text.contains(&rev)
    {
        return Err(format!("{} was not installed from the pinned revision {rev}", bin.display()));
    }
    Ok(bin)
}

/// A running daemon in its own directory; stopped and removed on drop.
#[derive(Debug)]
pub struct LiveDaemon {
    rf: PathBuf,
    dir: PathBuf,
    child: Option<Child>,
}

impl LiveDaemon {
    /// Starts a daemon in `/tmp/rf-f3-<pid>-<tag>` and waits until it answers.
    ///
    /// # Errors
    ///
    /// Returns why it did not start (no binary, no answer in time), with
    /// the daemon's log.
    pub fn start(tag: &str) -> Result<Self, String> {
        let dir = PathBuf::from(format!("/tmp/rf-f3-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let mut daemon = Self { rf: rf_binary()?, dir, child: None };
        daemon.spawn()?;
        Ok(daemon)
    }

    /// The daemon's local-bus socket.
    pub fn socket(&self) -> PathBuf {
        self.dir.join("rfd.sock")
    }

    /// The daemon's directory (socket, data root, log).
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn spawn(&mut self) -> Result<(), String> {
        let log = File::options()
            .create(true)
            .append(true)
            .open(self.dir.join("rfd.log"))
            .map_err(|e| format!("cannot open the daemon log: {e}"))?;
        let err = log.try_clone().map_err(|e| format!("cannot open the daemon log: {e}"))?;
        let child = Command::new(&self.rf)
            .arg("--socket")
            .arg(self.socket())
            .args(["daemon", "start", "--data-root"])
            .arg(self.dir.join("data"))
            .stdin(Stdio::null())
            .stdout(log)
            .stderr(err)
            .spawn()
            .map_err(|e| format!("cannot run {}: {e}", self.rf.display()))?;
        self.child = Some(child);
        let deadline = Instant::now() + READY_TIMEOUT;
        while Instant::now() < deadline {
            if self.rf(&["daemon", "status"]).is_ok_and(|out| out.status.success()) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(format!("the daemon did not answer in time; log:\n{}", self.log()))
    }

    /// Runs `rf --socket <socket> <args>` (to cross-check what the desktop
    /// shows against the reference CLI).
    ///
    /// # Errors
    ///
    /// Returns the I/O error when the binary cannot run.
    pub fn rf(&self, args: &[&str]) -> std::io::Result<Output> {
        Command::new(&self.rf).arg("--socket").arg(self.socket()).args(args).output()
    }

    /// The daemon's log so far.
    pub fn log(&self) -> String {
        fs::read_to_string(self.dir.join("rfd.log")).unwrap_or_default()
    }

    /// Stops the daemon and waits for it: `rf daemon stop` first (a
    /// graceful `daemon.stop`), then a terminate signal, then by force.
    pub fn stop(&mut self) {
        if self.child.is_some() {
            let _ = self.rf(&["daemon", "stop"]);
            if !self.wait_exit(Duration::from_secs(5))
                && let Some(child) = self.child.as_ref()
            {
                let _ = Command::new("kill")
                    .args(["-TERM", &child.id().to_string()])
                    .stderr(Stdio::null())
                    .status();
                self.wait_exit(Duration::from_secs(3));
            }
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_file(self.socket());
    }

    /// Waits up to `limit` for the daemon process to exit by itself;
    /// returns whether it has (true when no process is held).
    pub fn wait_exit(&mut self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            match self.child.as_mut().map(Child::try_wait) {
                None | Some(Ok(Some(_)) | Err(_)) => return true,
                Some(Ok(None)) if Instant::now() >= deadline => return false,
                Some(Ok(None)) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    /// The exit status of a daemon that has exited (and was not reaped by
    /// [`LiveDaemon::stop`]).
    pub fn exit_status(&mut self) -> Option<std::process::ExitStatus> {
        self.child.as_mut().and_then(|child| child.try_wait().ok().flatten())
    }

    /// Whether the daemon process is still running.
    pub fn is_running(&mut self) -> bool {
        self.child.as_mut().is_some_and(|child| child.try_wait().is_ok_and(|s| s.is_none()))
    }

    /// Starts the daemon again on the same socket and data root.
    ///
    /// # Errors
    ///
    /// As [`LiveDaemon::start`].
    pub fn restart(&mut self) -> Result<(), String> {
        self.stop();
        self.spawn()
    }
}

impl Drop for LiveDaemon {
    fn drop(&mut self) {
        self.stop();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pinned_revision_is_read_from_the_lockfile() {
        let rev = pinned_product_rev().expect("ramflux-sdk is a git dependency");
        assert_eq!(rev.len(), 40, "a full commit: {rev}");
        assert!(rev.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
