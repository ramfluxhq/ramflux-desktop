// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! How the SDK source reaches the daemon and how often it pulls.

use std::path::PathBuf;
use std::time::Duration;

pub use crate::bus::deadline::Deadlines;

/// Default daemon socket (the reference CLI's default).
pub const DEFAULT_SOCKET: &str = "/tmp/ramflux/rfd.sock";

/// Settings of the SDK source.
#[derive(Debug, Clone)]
pub struct SdkConfig {
    /// The daemon's local-bus socket.
    pub socket_path: PathBuf,
    /// Longest wait for the daemon socket to accept a connection.
    pub connect_timeout: Duration,
    /// Deadlines of requests, by what the method does.
    pub deadlines: Deadlines,
    /// Poll interval right after something changed.
    pub poll_fast: Duration,
    /// Poll interval after a quiet period (the interval doubles up to it).
    pub poll_slow: Duration,
    /// Wait before re-attaching after the daemon went away.
    pub reconnect_backoff: Duration,
    /// The desktop settings file holding pinned contexts; `None` keeps pins
    /// in memory only.
    pub settings_path: Option<PathBuf>,
}

impl SdkConfig {
    /// Defaults for `socket_path`.
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
            connect_timeout: Duration::from_secs(5),
            deadlines: Deadlines::standard(),
            poll_fast: Duration::from_millis(1_500),
            poll_slow: Duration::from_secs(12),
            reconnect_backoff: Duration::from_secs(2),
            settings_path: None,
        }
    }
}

impl Default for SdkConfig {
    fn default() -> Self {
        Self::new(DEFAULT_SOCKET)
    }
}
