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

/// The folder daemon directories are made in (default `/tmp`). Keep it
/// short: the socket path must fit a Unix socket address.
pub const ROOT_ENV: &str = "RAMFLUX_LIVE_DAEMON_ROOT";

/// When `1`, a dropped daemon is stopped but its directory (data, log) is
/// kept, for evidence.
pub const KEEP_ENV: &str = "RAMFLUX_LIVE_DAEMON_KEEP";

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
    /// Starts a daemon in `<root>/rf-f3-<pid>-<tag>` (root: [`ROOT_ENV`],
    /// else `/tmp`) and waits until it answers.
    ///
    /// # Errors
    ///
    /// Returns why it did not start (no binary, no answer in time), with
    /// the daemon's log.
    pub fn start(tag: &str) -> Result<Self, String> {
        let root = std::env::var_os(ROOT_ENV).map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
        let dir = root.join(format!("rf-f3-{}-{tag}", std::process::id()));
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
            .arg(flag("--socket", &self.socket()))
            .args(["daemon", "start"])
            .arg(flag("--data-root", &self.dir.join("data")))
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

    /// Runs `rf --socket=<socket> <args>` (to cross-check what the desktop
    /// shows against the reference CLI).
    ///
    /// # Errors
    ///
    /// Returns the I/O error when the binary cannot run.
    pub fn rf(&self, args: &[&str]) -> std::io::Result<Output> {
        Command::new(&self.rf).arg(flag("--socket", &self.socket())).args(args).output()
    }

    /// One local-bus request (`ramflux.local_bus.v1`, a 4-byte big-endian
    /// length and the JSON frame) on a fresh connection, for a test that
    /// plays a participant without a desktop (an AI assignee). Returns the
    /// answer's body, or the error frame's `code: message`.
    ///
    /// # Errors
    ///
    /// The daemon's refusal, or why the request could not be made.
    pub fn call(
        &self,
        account: Option<&str>,
        sdk_api: &str,
        method: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        use std::io::{Read as _, Write as _};
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = format!("live-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        let frame = serde_json::json!({
            "bus_protocol": "ramflux.local_bus.v1", "frame_id": format!("frame_{id}"),
            "kind": "request", "request_id": id, "account_id": account, "sdk_api": sdk_api,
            "method": method, "body": body, "trace_id": null,
        });
        let bytes = serde_json::to_vec(&frame).map_err(|e| e.to_string())?;
        let length = u32::try_from(bytes.len()).map_err(|e| e.to_string())?;
        let mut stream = std::os::unix::net::UnixStream::connect(self.socket())
            .map_err(|e| format!("cannot connect to the daemon: {e}"))?;
        stream.set_read_timeout(Some(Duration::from_secs(60))).map_err(|e| e.to_string())?;
        stream.write_all(&length.to_be_bytes()).map_err(|e| e.to_string())?;
        stream.write_all(&bytes).map_err(|e| e.to_string())?;
        loop {
            let mut len = [0_u8; 4];
            stream.read_exact(&mut len).map_err(|e| format!("{method}: no answer: {e}"))?;
            let mut answer = vec![0_u8; u32::from_be_bytes(len) as usize];
            stream.read_exact(&mut answer).map_err(|e| format!("{method}: no answer: {e}"))?;
            let answer: serde_json::Value =
                serde_json::from_slice(&answer).map_err(|e| e.to_string())?;
            if answer["request_id"] != id.as_str() {
                continue;
            }
            if answer["kind"] == "error" || answer["ok"] == false {
                let error = &answer["error"];
                return Err(format!(
                    "{method}: {}: {}",
                    error["code"].as_str().unwrap_or("error"),
                    error["message"].as_str().unwrap_or_default()
                ));
            }
            return Ok(answer["body"].clone());
        }
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
        if std::env::var(KEEP_ENV).ok().as_deref() != Some("1") {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }
}

/// `--name=value` for a path value, as one argument (a value is never read
/// as a flag).
fn flag(name: &str, value: &Path) -> std::ffi::OsString {
    let mut flag = std::ffi::OsString::from(format!("{name}="));
    flag.push(value);
    flag
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
