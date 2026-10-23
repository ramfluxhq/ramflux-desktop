// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The desktop's footprint on this machine, outside the daemon: where its
//! files live, the settings file, the launch settings, a local daemon the
//! user asked the desktop to start, and the application log.
//!
//! Everything here is plain `std`; no async runtime is involved.
//!
//! - [`paths`]: state, log and default data directories; the bundled daemon.
//! - [`settings_file`]: the versioned desktop settings file.
//! - [`launch`]: source, socket, daemon binary and data directory choices.
//! - [`daemon_process`]: a daemon process the desktop started and owns.
//! - [`app_log`]: the redacted, rotated application log.

pub mod app_log;
pub mod daemon_process;
pub mod launch;
pub mod paths;
pub mod settings_file;

pub use daemon_process::{DaemonCommand, DaemonProcess, LaunchError};
pub use launch::{DaemonBinary, LaunchSettings, LaunchStore, SourceKind};
pub use settings_file::{DesktopSettings, SETTINGS_SCHEMA, SettingsError};
