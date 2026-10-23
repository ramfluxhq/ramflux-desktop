// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The local daemon launcher and the launch settings: the application-layer
//! model behind "Start local daemon" and Settings › Diagnostics.
//!
//! [`DaemonLauncher`] never starts a daemon on its own. Only
//! [`DaemonLauncher::start`], reached from the user's "Start local daemon"
//! command, runs `rf daemon start` on the socket the live source already
//! watches; the source then attaches by itself as soon as the daemon
//! answers. A daemon the launcher started is owned by it: it is stopped by
//! terminating the child process (the pinned CLI cannot stop a foreground
//! daemon), on the user's "Stop" command and when the application quits.
//!
//! Launching a process is a local application action, not a domain intent:
//! it involves no account, no daemon method and no policy, so it travels as
//! a window command handled by the shell, not through the intent root.
//!
//! - [`choice`]: the source chosen by flags, environment and settings.
//! - [`version`]: version information shown in diagnostics.

pub mod choice;
pub mod version;

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::{App, Context, Entity, Task};
use ramflux_desktop_core::SourceStatus;
use ramflux_desktop_sdk::local::{
    DaemonBinary, DaemonCommand, DaemonProcess, LaunchError, LaunchSettings, LaunchStore,
    SourceKind, app_log, daemon_process, paths,
};

use crate::app::StoreHandle;

/// Interval at which a started daemon is checked.
pub const POLL: Duration = Duration::from_millis(250);

/// Polls a stopping daemon gets before it is killed.
const STOP_POLLS: u32 = 12;

/// What the launcher has done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchState {
    /// Nothing started by the desktop.
    Idle,
    /// Started; the daemon does not answer yet.
    Starting {
        /// Process id.
        pid: u32,
    },
    /// Started and answering.
    Running {
        /// Process id.
        pid: u32,
    },
    /// Asked to stop; not exited yet.
    Stopping {
        /// Process id.
        pid: u32,
    },
    /// Not started, or exited on its own.
    Failed(LaunchFailure),
}

/// Why a daemon the user asked for is not running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchFailure {
    /// It could not be started.
    Launch(LaunchError),
    /// It exited by itself, with this exit code (none when ended by a
    /// signal).
    Exited(Option<i32>),
}

/// Starts, watches and stops a local daemon; keeps the launch settings.
pub struct DaemonLauncher {
    store: Entity<StoreHandle>,
    settings_store: LaunchStore,
    settings: LaunchSettings,
    bundled: Option<PathBuf>,
    log_dir: Option<PathBuf>,
    process: Option<DaemonProcess>,
    state: LaunchState,
    saved: Option<Result<(), String>>,
    stop_polls: u32,
    poll: Option<Task<()>>,
}

/// Where a launcher keeps its settings, finds a bundled daemon and writes
/// daemon output.
#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    /// The settings file part.
    pub store: LaunchStore,
    /// A daemon binary shipped with the application.
    pub bundled: Option<PathBuf>,
    /// Directory of the daemon log.
    pub log_dir: Option<PathBuf>,
}

impl LaunchOptions {
    /// Nothing persisted, nothing bundled, no log (tests, screenshots).
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// The application's: the platform settings file, a daemon bundled next
    /// to the executable, the platform log directory.
    pub fn application() -> Self {
        Self {
            store: LaunchStore::default_location(),
            bundled: paths::bundled_daemon(),
            log_dir: paths::log_dir(),
        }
    }
}

impl DaemonLauncher {
    /// A launcher for the daemon `store` watches, set up by `options`.
    pub fn new(store: Entity<StoreHandle>, options: LaunchOptions, cx: &mut Context<Self>) -> Self {
        // A daemon the desktop owns stops with the application.
        cx.on_app_quit(|this: &mut Self, _| {
            this.stop_now();
            async {}
        })
        .detach();
        Self {
            store,
            settings: options.store.load(),
            settings_store: options.store,
            bundled: options.bundled,
            log_dir: options.log_dir,
            process: None,
            state: LaunchState::Idle,
            saved: None,
            stop_polls: 0,
            poll: None,
        }
    }

    /// The saved launch settings.
    pub fn settings(&self) -> &LaunchSettings {
        &self.settings
    }

    /// What the launcher has done.
    pub fn state(&self) -> &LaunchState {
        &self.state
    }

    /// Whether a daemon started by the desktop is running (or stopping).
    pub fn owns_daemon(&self) -> bool {
        self.process.is_some()
    }

    /// The result of the last save, if any.
    pub fn last_save(&self) -> Option<&Result<(), String>> {
        self.saved.as_ref()
    }

    /// A daemon binary shipped with the application, if any.
    pub fn bundled(&self) -> Option<&std::path::Path> {
        self.bundled.as_deref()
    }

    /// The binary a start would run.
    pub fn binary(&self) -> DaemonBinary {
        self.settings.daemon_binary_with(self.bundled.clone())
    }

    /// The data directory a start would use.
    pub fn data_dir(&self) -> Option<PathBuf> {
        self.settings.data_dir_or_default()
    }

    /// The file a started daemon writes its output to.
    pub fn daemon_log(&self) -> Option<PathBuf> {
        self.log_dir.as_ref().map(|dir| dir.join(app_log::DAEMON_LOG_FILE))
    }

    /// The settings file, if the settings persist.
    pub fn settings_file(&self) -> Option<PathBuf> {
        self.settings_store.path().map(PathBuf::from)
    }

    /// The socket the live source watches (and a start listens on); `None`
    /// when the window plays scenarios.
    pub fn socket(&self, cx: &App) -> Option<PathBuf> {
        self.store.read(cx).live_socket().map(PathBuf::from)
    }

    /// Whether "Start local daemon" can be offered now: live, not attached,
    /// and nothing started or starting.
    pub fn can_start(&self, cx: &App) -> bool {
        let handle = self.store.read(cx);
        handle.is_live()
            && handle.store().source_status() == SourceStatus::Disconnected
            && self.process.is_none()
    }

    /// Starts the daemon on the watched socket. Called only on the user's
    /// command; does nothing when a start is not possible now.
    pub fn start(&mut self, cx: &mut Context<Self>) {
        if !self.can_start(cx) {
            return;
        }
        let Some(socket) = self.socket(cx) else { return };
        let result = self.command(socket).and_then(|command| DaemonProcess::spawn(&command));
        match result {
            Ok(process) => {
                let pid = process.pid();
                app_log::info(&format!("local daemon started on request pid={pid}"));
                self.process = Some(process);
                self.state = LaunchState::Starting { pid };
                self.watch(cx);
            }
            Err(error) => {
                app_log::warn(&format!("local daemon not started: {error}"));
                self.state = LaunchState::Failed(LaunchFailure::Launch(error));
            }
        }
        cx.notify();
    }

    fn command(&self, socket: PathBuf) -> Result<DaemonCommand, LaunchError> {
        let binary = self.binary().path().map(PathBuf::from).ok_or(LaunchError::BinaryMissing)?;
        daemon_process::check_binary(&binary)?;
        let data_dir = self.data_dir().ok_or(LaunchError::NoDataDir)?;
        Ok(DaemonCommand { binary, socket, data_dir, log_file: self.daemon_log() })
    }

    /// Asks a daemon the desktop started to stop; it is killed if it does
    /// not exit within a few seconds. Never touches a daemon it did not
    /// start.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        let Some(process) = self.process.as_mut() else { return };
        process.terminate();
        let pid = process.pid();
        app_log::info(&format!("local daemon stop requested pid={pid}"));
        self.state = LaunchState::Stopping { pid };
        self.stop_polls = 0;
        self.watch(cx);
        cx.notify();
    }

    /// Stops a daemon the desktop started and waits for it (application
    /// quit).
    pub fn stop_now(&mut self) {
        if let Some(mut process) = self.process.take() {
            app_log::info(&format!("local daemon stopped on quit pid={}", process.pid()));
            process.stop(daemon_process::STOP_GRACE);
            self.state = LaunchState::Idle;
        }
    }

    fn watch(&mut self, cx: &mut Context<Self>) {
        if self.poll.is_some() {
            return;
        }
        self.poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                let watching = this.update(cx, |this, cx| this.check(cx)).unwrap_or(false);
                if !watching {
                    break;
                }
            }
        }));
    }

    /// One check of the started daemon; returns whether to keep watching.
    fn check(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(process) = self.process.as_mut() else {
            self.poll = None;
            return false;
        };
        let pid = process.pid();
        if let Some(code) = process.exit_code() {
            self.process = None;
            self.poll = None;
            self.state = if matches!(self.state, LaunchState::Stopping { .. }) {
                app_log::info(&format!("local daemon stopped pid={pid}"));
                LaunchState::Idle
            } else {
                let status = code.map_or_else(|| "signal".to_owned(), |c| c.to_string());
                app_log::warn(&format!("local daemon exited pid={pid} status {status}"));
                LaunchState::Failed(LaunchFailure::Exited(code))
            };
            cx.notify();
            return false;
        }
        let attached = self.store.read(cx).store().source_status() != SourceStatus::Disconnected;
        match self.state {
            LaunchState::Stopping { .. } => {
                self.stop_polls += 1;
                if self.stop_polls > STOP_POLLS {
                    app_log::warn(&format!("local daemon killed after no exit pid={pid}"));
                    process.kill();
                    self.process = None;
                    self.poll = None;
                    self.state = LaunchState::Idle;
                    cx.notify();
                    return false;
                }
            }
            LaunchState::Starting { pid } if attached => {
                app_log::info(&format!("local daemon answering pid={pid}"));
                self.state = LaunchState::Running { pid };
                cx.notify();
            }
            _ => {}
        }
        true
    }

    /// Saves `settings`. Daemon binary and data directory apply to the next
    /// start; source and socket to the next application start.
    pub fn save(&mut self, settings: LaunchSettings, cx: &mut Context<Self>) {
        let result = self.settings_store.save(&settings).map_err(|e| e.to_string());
        match &result {
            Ok(()) => {
                app_log::info("launch settings saved");
                self.settings = settings;
            }
            Err(error) => app_log::warn(&format!("launch settings not saved: {error}")),
        }
        self.saved = Some(result);
        cx.notify();
    }

    /// Saves the source choice alone.
    pub fn set_source(&mut self, source: SourceKind, cx: &mut Context<Self>) {
        let settings = LaunchSettings { source: Some(source), ..self.settings.clone() };
        self.save(settings, cx);
    }
}
