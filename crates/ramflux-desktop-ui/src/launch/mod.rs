// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The local daemon launcher and the launch settings: the application-layer
//! model behind "Start local daemon" and Settings › Diagnostics.
//!
//! [`DaemonLauncher`] never starts a daemon on its own. Only
//! [`DaemonLauncher::start`], reached from the user's "Start local daemon"
//! command, runs `rf daemon start` on the socket the live source already
//! watches; the source then attaches by itself as soon as the daemon
//! answers. A daemon the launcher started is owned by it: it is stopped on
//! the user's "Stop" command and when the application quits, gracefully over
//! the local bus (`daemon.stop`) with a bounded fallback to terminating the
//! child process (see [`stopping`]). A daemon left behind by a session that
//! crashed is recognised by its record and can be stopped gracefully too; a
//! daemon the user started without the desktop is never offered.
//!
//! Launching a process is a local application action, not a domain intent:
//! it involves no account, no daemon method and no policy, so it travels as
//! a window command handled by the shell, not through the intent root.
//!
//! - [`choice`]: the source chosen by flags, environment and settings.
//! - [`stopping`]: the bounded stop sequence of an owned daemon.
//! - [`version`]: version information shown in diagnostics.

pub mod choice;
pub mod stopping;
pub mod version;

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::{App, Context, Entity, Task};
use ramflux_desktop_core::SourceStatus;
use ramflux_desktop_sdk::DaemonStop;
use ramflux_desktop_sdk::local::{
    DaemonBinary, DaemonCommand, DaemonProcess, LaunchError, LaunchSettings, LaunchStore,
    OwnedDaemon, OwnedDaemonFile, SourceKind, app_log, daemon_process, paths,
};

use crate::app::StoreHandle;
use stopping::{GRACEFUL_POLLS, StopSequence, StopStep};

/// Interval at which a started daemon is checked.
pub const POLL: Duration = Duration::from_millis(250);

/// How long quitting waits for the daemon to accept a graceful stop.
const QUIT_ANSWER_WAIT: Duration = Duration::from_secs(2);

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
    /// Asked to stop (an owned daemon, or one left behind); not gone yet.
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
    /// A daemon left behind by an earlier session refused to stop, or did
    /// not stop in time (the reason).
    Stop(String),
}

/// The graceful stop of a daemon left behind by an earlier session (no
/// process handle: it can only be asked).
#[derive(Debug)]
struct LeftBehindStop {
    request: Option<DaemonStop>,
    accepted: bool,
    polls: u32,
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
    owned_file: OwnedDaemonFile,
    left_behind: Option<OwnedDaemon>,
    stopping: Option<StopSequence>,
    stopping_left_behind: Option<LeftBehindStop>,
    last_stop_graceful: Option<bool>,
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
    /// The record of a started daemon (to find one left behind by a crash).
    pub owned: OwnedDaemonFile,
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
            owned: OwnedDaemonFile::default_location(),
        }
    }
}

impl DaemonLauncher {
    /// A launcher for the daemon `store` watches, set up by `options`.
    pub fn new(store: Entity<StoreHandle>, options: LaunchOptions, cx: &mut Context<Self>) -> Self {
        // A daemon the desktop owns stops with the application.
        cx.on_app_quit(|this: &mut Self, cx| {
            this.stop_now(cx);
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
            left_behind: options.owned.load(),
            owned_file: options.owned,
            stopping: None,
            stopping_left_behind: None,
            last_stop_graceful: None,
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

    /// The process id of a daemon an earlier session of the desktop started
    /// and did not stop (it crashed), when that daemon is the one attached
    /// now; `None` otherwise.
    pub fn left_behind(&self, cx: &App) -> Option<u32> {
        let record = self.left_behind.as_ref()?;
        let handle = self.store.read(cx);
        let attached = handle.store().source_status() != SourceStatus::Disconnected;
        let same_socket = handle.live_socket().map(PathBuf::from).as_ref() == Some(&record.socket);
        (self.process.is_none() && attached && same_socket && self.state == LaunchState::Idle)
            .then_some(record.pid)
    }

    /// Whether the last stop of a daemon the desktop started ended it
    /// gracefully (`daemon.stop`) rather than by signal; `None` before any.
    pub fn last_stop_graceful(&self) -> Option<bool> {
        self.last_stop_graceful
    }

    /// Whether "Stop daemon" can be offered now: a daemon the desktop
    /// started, or one an earlier session left behind.
    pub fn can_stop(&self, cx: &App) -> bool {
        (self.process.is_some() && !matches!(self.state, LaunchState::Stopping { .. }))
            || self.left_behind(cx).is_some()
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
        let result =
            self.command(socket.clone()).and_then(|command| DaemonProcess::spawn(&command));
        match result {
            Ok(process) => {
                let pid = process.pid();
                app_log::info(&format!("local daemon started on request pid={pid}"));
                self.owned_file.record(&OwnedDaemon { pid, socket: socket.clone() });
                self.left_behind = None;
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

    /// Stops a daemon the desktop started (gracefully, then by signal, then
    /// by force, each step bounded), or asks one an earlier session left
    /// behind to stop. Never touches a daemon the user started without the
    /// desktop.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if !self.can_stop(cx) {
            return;
        }
        let request = self.store.read(cx).request_daemon_stop();
        if let Some(process) = self.process.as_ref() {
            let pid = process.pid();
            app_log::info(&format!("local daemon graceful stop requested pid={pid}"));
            self.stopping = Some(StopSequence::new(request));
            self.state = LaunchState::Stopping { pid };
        } else if let Some(pid) = self.left_behind(cx) {
            app_log::info(&format!("left-behind daemon graceful stop requested pid={pid}"));
            self.stopping_left_behind = Some(LeftBehindStop { request, accepted: false, polls: 0 });
            self.state = LaunchState::Stopping { pid };
        }
        self.watch(cx);
        cx.notify();
    }

    /// Stops a daemon the desktop started and waits for it (application
    /// quit): a graceful request first, then the bounded fallback.
    pub fn stop_now(&mut self, cx: &App) {
        let Some(mut process) = self.process.take() else { return };
        let pid = process.pid();
        let accepted = self
            .store
            .read(cx)
            .request_daemon_stop()
            .and_then(|mut request| request.wait(QUIT_ANSWER_WAIT))
            .is_some_and(|answer| answer.is_ok());
        let graceful = accepted && process.wait_exit(daemon_process::STOP_GRACE);
        self.last_stop_graceful = Some(graceful);
        if graceful {
            app_log::info(&format!("local daemon stopped gracefully on quit pid={pid}"));
        } else {
            app_log::info(&format!("local daemon terminated on quit pid={pid}"));
            process.stop(daemon_process::STOP_GRACE);
        }
        self.owned_file.clear();
        self.stopping = None;
        self.state = LaunchState::Idle;
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

    /// One check of the started (or stopping) daemon; returns whether to
    /// keep watching.
    fn check(&mut self, cx: &mut Context<Self>) -> bool {
        let watching =
            if self.process.is_some() { self.check_owned(cx) } else { self.check_left_behind(cx) };
        if !watching {
            self.poll = None;
            cx.notify();
        }
        watching
    }

    fn check_owned(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(process) = self.process.as_mut() else { return false };
        let pid = process.pid();
        if let Some(code) = process.exit_code() {
            self.process = None;
            self.owned_file.clear();
            let stopping = self.stopping.take();
            self.state = match stopping {
                Some(sequence) => {
                    self.last_stop_graceful = Some(sequence.graceful());
                    let how = if sequence.graceful() { "gracefully" } else { "after a signal" };
                    app_log::info(&format!("local daemon stopped {how} pid={pid}"));
                    LaunchState::Idle
                }
                None => {
                    let status = code.map_or_else(|| "signal".to_owned(), |c| c.to_string());
                    app_log::warn(&format!("local daemon exited pid={pid} status {status}"));
                    LaunchState::Failed(LaunchFailure::Exited(code))
                }
            };
            return false;
        }
        if let Some(sequence) = self.stopping.as_mut() {
            match sequence.step() {
                StopStep::Wait => {}
                StopStep::Terminate => {
                    let why = sequence.refused().unwrap_or("no exit in time").to_owned();
                    app_log::warn(&format!("local daemon terminated pid={pid}: {why}"));
                    process.terminate();
                }
                StopStep::Kill => {
                    app_log::warn(&format!("local daemon killed after no exit pid={pid}"));
                    process.kill();
                    self.process = None;
                    self.stopping = None;
                    self.last_stop_graceful = Some(false);
                    self.owned_file.clear();
                    self.state = LaunchState::Idle;
                    return false;
                }
            }
            return true;
        }
        let attached = self.store.read(cx).store().source_status() != SourceStatus::Disconnected;
        if let LaunchState::Starting { pid } = self.state
            && attached
        {
            app_log::info(&format!("local daemon answering pid={pid}"));
            self.state = LaunchState::Running { pid };
            cx.notify();
        }
        true
    }

    fn check_left_behind(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(stop) = self.stopping_left_behind.as_mut() else { return false };
        stop.polls += 1;
        let answer = stop
            .request
            .as_mut()
            .map_or_else(|| Some(Err("no daemon link".to_owned())), DaemonStop::poll);
        let failed = match answer {
            Some(Ok(())) => {
                stop.accepted = true;
                None
            }
            Some(Err(reason)) if !stop.accepted => Some(reason),
            _ if stop.polls > GRACEFUL_POLLS => Some("the daemon kept answering".to_owned()),
            _ => None,
        };
        let gone = self.store.read(cx).store().source_status() == SourceStatus::Disconnected;
        if let Some(reason) = failed {
            app_log::warn(&format!("left-behind daemon did not stop: {reason}"));
            self.stopping_left_behind = None;
            self.state = LaunchState::Failed(LaunchFailure::Stop(reason));
            return false;
        }
        if stop.accepted && gone {
            app_log::info("left-behind daemon stopped gracefully");
            self.stopping_left_behind = None;
            self.left_behind = None;
            self.owned_file.clear();
            self.state = LaunchState::Idle;
            return false;
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

    /// Saves whether polling stands in for a missing event stream (applies
    /// the next time the application connects to the daemon).
    pub fn set_poll_fallback(&mut self, on: bool, cx: &mut Context<Self>) {
        let settings = LaunchSettings { poll_fallback: Some(on), ..self.settings.clone() };
        self.save(settings, cx);
    }

    /// Saves the source choice alone.
    pub fn set_source(&mut self, source: SourceKind, cx: &mut Context<Self>) {
        let settings = LaunchSettings { source: Some(source), ..self.settings.clone() };
        self.save(settings, cx);
    }
}
