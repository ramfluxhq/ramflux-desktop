// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! First run on the daemon source: no daemon answers, so the entry page
//! offers "Start local daemon" and starts nothing by itself; a start runs the
//! configured program on the watched socket and can be stopped; a missing
//! program disables the offer and points to Settings › Diagnostics; a daemon
//! that exits at once is reported; launch settings typed in Settings ›
//! Diagnostics persist.
//!
//! A shell script stands in for `rf` (it records its arguments and waits),
//! so no real daemon runs here.

#![cfg(unix)]

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Entity, ScrollDelta, TestAppContext, point, px};
use ramflux_desktop_sdk::SdkConfig;
use ramflux_desktop_sdk::local::{LaunchError, LaunchStore};
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::launch::{DaemonLauncher, LaunchFailure, LaunchOptions, LaunchState};
use ramflux_desktop_ui::route::{EntryStep, Page};
use ramflux_desktop_ui::settings::SettingsSection;
use ramflux_desktop_ui::shell::{ShellOptions, SourceChoice};

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rf-f5-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        Self { dir }
    }

    /// A stand-in `rf`: records its arguments, then runs `then`.
    fn program(&self, then: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let path = self.dir.join("rf");
        let script =
            format!("#!/bin/sh\necho \"$@\" > '{}'\n{then}\n", self.dir.join("args").display());
        std::fs::write(&path, script).expect("script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("mode");
        path
    }

    fn socket(&self) -> PathBuf {
        self.dir.join("rfd.sock")
    }

    fn args(&self) -> Option<String> {
        std::fs::read_to_string(self.dir.join("args")).ok()
    }

    fn options(&self, bundled: Option<PathBuf>, settings: LaunchStore) -> ShellOptions {
        let config = SdkConfig {
            reconnect_backoff: Duration::from_millis(100),
            ..SdkConfig::new(self.socket())
        };
        ShellOptions {
            source: SourceChoice::Rfd(config),
            launch: LaunchOptions {
                store: settings,
                bundled,
                log_dir: Some(self.dir.join("logs")),
            },
            ..ShellOptions::for_scenario("", true)
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn launcher(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Entity<DaemonLauncher> {
    cx.update(|cx| shell.read(cx).launcher().clone())
}

fn state(cx: &mut TestAppContext, launcher: &Entity<DaemonLauncher>) -> LaunchState {
    cx.update(|cx| launcher.read(cx).state().clone())
}

/// Lets the child process run while the GPUI test clock ticks.
fn wait(cx: &mut TestAppContext, what: &str, mut done: impl FnMut(&mut TestAppContext) -> bool) {
    for _ in 0..300 {
        if done(cx) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
        advance(cx, 100);
    }
    panic!("timed out waiting for: {what}");
}

#[gpui_kit::test]
fn no_daemon_offers_a_start_and_starts_nothing_by_itself(cx: &mut TestAppContext) {
    let fx = Fixture::new("offer");
    let program = fx.program("exec sleep 30");
    let (handle, shell) =
        open_with(cx, fx.options(Some(program.clone()), LaunchStore::in_memory()));
    let launcher = launcher(cx, &shell);
    advance(cx, 3_000);
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Daemon));
    for id in ["entry-daemon-start", "entry-daemon-settings", "entry-daemon-binary"] {
        assert!(exists(cx, handle, id), "{id} is offered");
    }
    assert!(!exists(cx, handle, "entry-daemon-missing"));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(state(cx, &launcher), LaunchState::Idle, "nothing starts without the user");
    assert_eq!(fx.args(), None, "the program never ran");

    click(cx, handle, "entry-daemon-start");
    assert!(matches!(state(cx, &launcher), LaunchState::Starting { .. }));
    assert!(exists(cx, handle, "entry-daemon-starting"), "the start shows its progress");
    wait(cx, "the program ran", |_| fx.args().is_some());
    let args = fx.args().unwrap_or_default();
    assert!(
        args.starts_with(&format!("--socket {} daemon start --data-root", fx.socket().display())),
        "started on the watched socket: {args}"
    );
    let first = state(cx, &launcher);
    cx.update(|cx| launcher.update(cx, |launcher, cx| launcher.start(cx)));
    assert_eq!(state(cx, &launcher), first, "a second start request starts nothing");

    cx.update(|cx| launcher.update(cx, |launcher, cx| launcher.stop(cx)));
    wait(cx, "stopped", |cx| state(cx, &launcher) == LaunchState::Idle);
    assert!(!cx.update(|cx| launcher.read(cx).owns_daemon()));
    assert!(exists(cx, handle, "entry-daemon-start"), "the offer comes back");
}

#[gpui_kit::test]
fn a_missing_program_disables_the_start_and_points_to_settings(cx: &mut TestAppContext) {
    let fx = Fixture::new("missing");
    let (handle, shell) = open_with(cx, fx.options(None, LaunchStore::in_memory()));
    let launcher = launcher(cx, &shell);
    advance(cx, 500);
    assert!(exists(cx, handle, "entry-daemon-missing"), "the missing program is explained");
    assert!(!exists(cx, handle, "entry-daemon-binary"));
    click(cx, handle, "entry-daemon-start");
    assert_eq!(state(cx, &launcher), LaunchState::Idle, "a disabled start does nothing");
    click(cx, handle, "entry-daemon-settings");
    assert!(cx.update(|cx| shell.read(cx).is_settings_open()));
    let section = cx.update(|cx| shell.read(cx).settings().read(cx).section());
    assert_eq!(section, SettingsSection::Diagnostics);
    assert!(exists(cx, handle, "renderer-profile"), "diagnostics is shown");
    // A start requested anyway (command palette, key binding) fails clearly.
    cx.update(|cx| launcher.update(cx, |launcher, cx| launcher.start(cx)));
    assert_eq!(
        state(cx, &launcher),
        LaunchState::Failed(LaunchFailure::Launch(LaunchError::BinaryMissing))
    );
}

#[gpui_kit::test]
fn a_daemon_that_exits_at_once_is_reported_and_can_be_retried(cx: &mut TestAppContext) {
    let fx = Fixture::new("exit");
    let program = fx.program("exit 3");
    let (handle, shell) = open_with(cx, fx.options(Some(program), LaunchStore::in_memory()));
    let launcher = launcher(cx, &shell);
    advance(cx, 500);
    click(cx, handle, "entry-daemon-start");
    wait(cx, "exit seen", |cx| matches!(state(cx, &launcher), LaunchState::Failed(_)));
    assert_eq!(state(cx, &launcher), LaunchState::Failed(LaunchFailure::Exited(Some(3))));
    assert!(exists(cx, handle, "entry-daemon-failed"), "the failure is shown");
    assert!(exists(cx, handle, "entry-daemon-start"), "and can be retried");
}

#[gpui_kit::test]
fn a_configured_program_that_is_gone_is_named(cx: &mut TestAppContext) {
    let fx = Fixture::new("gone");
    let gone = fx.dir.join("no-such-rf");
    let (handle, shell) = open_with(cx, fx.options(Some(gone.clone()), LaunchStore::in_memory()));
    let launcher = launcher(cx, &shell);
    advance(cx, 500);
    click(cx, handle, "entry-daemon-start");
    assert_eq!(
        state(cx, &launcher),
        LaunchState::Failed(LaunchFailure::Launch(LaunchError::BinaryNotFound(gone)))
    );
    assert!(exists(cx, handle, "entry-daemon-failed"));
}

#[gpui_kit::test]
fn launch_settings_typed_in_diagnostics_persist(cx: &mut TestAppContext) {
    let fx = Fixture::new("save");
    let file = fx.dir.join("desktop-settings.json");
    let (handle, shell) = open_with(cx, fx.options(None, LaunchStore::at(file.clone())));
    let launcher = launcher(cx, &shell);
    click(cx, handle, "entry-daemon-settings");
    // The local setup follows the export and the renderer profile.
    in_window(cx, handle, |window, cx| {
        let delta = ScrollDelta::Pixels(point(px(0.), px(-900.)));
        window.scroll("renderer-profile", delta, cx);
    });
    type_into(cx, handle, "launch-binary", "/opt/ramflux/bin/rf");
    type_into(cx, handle, "launch-data-dir", "/tmp/rf-demo-data");
    click(cx, handle, "launch-save");
    assert!(exists(cx, handle, "launch-saved"), "saving is confirmed");
    let saved = LaunchStore::at(file).load();
    assert_eq!(saved.daemon_binary.as_deref(), Some(Path::new("/opt/ramflux/bin/rf")));
    assert_eq!(saved.data_dir.as_deref(), Some(Path::new("/tmp/rf-demo-data")));
    assert_eq!(saved.socket_path, None, "an empty field keeps the default");
    let binary = cx.update(|cx| launcher.read(cx).binary());
    assert_eq!(binary.path(), Some(Path::new("/opt/ramflux/bin/rf")), "used by the next start");
}

#[gpui_kit::test]
fn scenarios_show_the_wait_without_an_offer(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s01_first_launch");
    advance(cx, 1);
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Daemon));
    assert!(exists(cx, handle, "entry-daemon"));
    assert!(!exists(cx, handle, "entry-daemon-start"), "scenarios never start a daemon");
}
