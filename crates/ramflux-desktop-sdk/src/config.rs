// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! How the SDK source reaches the daemon, follows its event stream, and how
//! often it polls when the stream is unavailable.

use std::path::PathBuf;
use std::time::Duration;

pub use crate::bus::deadline::Deadlines;

/// Default daemon socket (the reference CLI's default).
pub const DEFAULT_SOCKET: &str = "/tmp/ramflux/rfd.sock";

/// Default waits, in milliseconds, before repeating a node lookup that
/// macOS Local Network privacy may have blocked: the system refreshes its
/// record of the app (or shows its prompt) on the blocked attempt, so a
/// repeat a moment later goes through.
pub const LOCAL_NETWORK_RETRY_MS: [u16; 2] = [2_000, 4_000];

/// Settings of the SDK source.
#[derive(Debug, Clone)]
pub struct SdkConfig {
    /// The daemon's local-bus socket.
    pub socket_path: PathBuf,
    /// Longest wait for the daemon socket to accept a connection.
    pub connect_timeout: Duration,
    /// Deadlines of requests, by what the method does.
    pub deadlines: Deadlines,
    /// Whether to poll when the daemon offers no event stream (or the
    /// stream keeps failing). Off, the source shows that it is not kept up
    /// to date instead.
    pub poll_fallback: bool,
    /// Wait before trying the event stream again after it kept failing.
    pub stream_retry: Duration,
    /// Poll interval right after something changed (polling only).
    pub poll_fast: Duration,
    /// Poll interval after a quiet period (the interval doubles up to it).
    pub poll_slow: Duration,
    /// Wait before re-attaching after the daemon went away.
    pub reconnect_backoff: Duration,
    /// The desktop settings file holding pinned contexts; `None` keeps pins
    /// in memory only.
    pub settings_path: Option<PathBuf>,
    /// Waits, in milliseconds, before each repeat of a node lookup that
    /// macOS Local Network privacy may have blocked; a zero ends the
    /// repeats. (Milliseconds keep the settings as small as before.)
    pub local_network_retry_ms: [u16; 2],
}

impl SdkConfig {
    /// Defaults for `socket_path`.
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
            connect_timeout: Duration::from_secs(5),
            deadlines: Deadlines::standard(),
            poll_fallback: true,
            stream_retry: Duration::from_secs(30),
            poll_fast: Duration::from_millis(1_500),
            poll_slow: Duration::from_secs(12),
            reconnect_backoff: Duration::from_secs(2),
            settings_path: None,
            local_network_retry_ms: LOCAL_NETWORK_RETRY_MS,
        }
    }
}

impl Default for SdkConfig {
    fn default() -> Self {
        Self::new(DEFAULT_SOCKET)
    }
}
