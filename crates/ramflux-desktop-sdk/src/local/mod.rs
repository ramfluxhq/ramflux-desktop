// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The desktop's footprint on this machine, outside the daemon: where its
//! files live, the settings file, the launch settings, a local daemon the
//! user asked the desktop to start, and the application log.
//!
//! Everything here is plain `std`; no async runtime is involved.
//!
//! - [`identity`]: the application id and names used by the platform.
//! - [`platform_dirs`]: the operating system's per-user directories.
//! - [`paths`]: settings, log and default data directories, the default
//!   socket; the bundled daemon.
//! - [`notifications`]: how much operating-system notifications reveal.
//! - [`settings_file`]: the versioned desktop settings file.
//! - [`launch`]: source, socket, daemon binary and data directory choices.
//! - [`appearance`]: theme and language choices of this device.
//! - [`daemon_process`]: a daemon process the desktop started and owns.
//! - [`owned_daemon`]: the record of a started daemon, to find one left
//!   behind by a crash.
//! - [`app_log`]: the redacted, rotated application log.

pub mod app_log;
pub mod appearance;
pub mod daemon_process;
pub mod identity;
pub mod launch;
pub mod notifications;
pub mod owned_daemon;
pub mod paths;
pub mod platform_dirs;
pub mod settings_file;

pub use appearance::{AppearanceSettings, AppearanceStore, FOLLOW_SYSTEM};
pub use daemon_process::{DaemonCommand, DaemonProcess, LaunchError};
pub use launch::{DaemonBinary, LaunchSettings, LaunchStore, SourceKind};
pub use notifications::{NotificationSettings, NotificationStore, PreviewLevel};
pub use owned_daemon::{OwnedDaemon, OwnedDaemonFile};
pub use settings_file::{DesktopSettings, SETTINGS_SCHEMA, SettingsError};
