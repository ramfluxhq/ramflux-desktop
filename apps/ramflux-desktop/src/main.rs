// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Ramflux Desktop entrypoint.
//!
//! ```text
//! ramflux-desktop [--source mock|rfd] [--socket PATH]
//! ```
//!
//! The source comes from the flags, then `RAMFLUX_DESKTOP_SOURCE` and
//! `RAMFLUX_DESKTOP_SOCKET`, then the launch settings saved in Settings ›
//! Diagnostics, then the build: the bundled scenarios in debug builds, the
//! local daemon at the platform's default socket in release builds. Without
//! the `demo` feature (release packaging) there are no scenarios: `mock` is
//! refused and the daemon is the only source.
//!
//! The desktop attaches to a daemon that is already running. It never
//! starts one by itself; the entry page offers "Start local daemon".
//! Application events go to the platform log directory, redacted.
//!
//! The window carries the application id `org.ramflux.desktop` (Wayland
//! `app_id`, X11 `WM_CLASS`), matching the Linux desktop entry. Before the
//! window opens, daemon data left by an older build under the Linux
//! settings directory is copied to the XDG data directory once
//! (`local::data_migration`), unless a data directory is set.

// Production code: no unwrap, expect, panic or `std::sync` locks (see the
// workspace `Cargo.toml`); tests may.
#![cfg_attr(
    not(test),
    deny(clippy::disallowed_types, clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

use gpui_kit::component::TitleBar;
use gpui_kit::*;
use ramflux_desktop_sdk::local::data_migration::{self, Outcome};
use ramflux_desktop_sdk::local::{AppearanceStore, LaunchStore, app_log, identity, paths};
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::launch::choice::{self, Overrides};
use ramflux_desktop_ui::launch::version;
use ramflux_desktop_ui::shell::SourceChoice;

/// Initial window size: a physical window boundary, not application layout.
const WINDOW_SIZE: (f32, f32) = (1280., 820.);

/// The source chosen by flags, then the environment, then the saved
/// settings, then the build.
fn source_choice() -> Result<SourceChoice, String> {
    let flags = Overrides::from_args(std::env::args().skip(1))?;
    let env = Overrides {
        source: std::env::var(choice::SOURCE_ENV).ok(),
        socket: std::env::var(choice::SOCKET_ENV).ok(),
    };
    choice::resolve(&flags.or(env), &LaunchStore::default_location().load())
}

/// Copies daemon data from the pre-XDG default folder once (Linux). A
/// configured data directory is the user's choice and is left alone.
fn migrate_daemon_data() {
    if LaunchStore::default_location().load().data_dir.is_some() {
        return;
    }
    let Some(dirs) = paths::data_dirs() else { return };
    let Some(legacy) = dirs.legacy.as_deref() else { return };
    match data_migration::migrate(legacy, &dirs.current) {
        Ok(Outcome::Migrated { files, bytes }) => app_log::info(&format!(
            "daemon data copied to {} ({files} files, {bytes} bytes); {} is kept and may be removed",
            dirs.current.display(),
            legacy.display()
        )),
        Ok(Outcome::InUse) => {
            app_log::info("daemon data not copied: a daemon is using it; next start")
        }
        Ok(Outcome::NothingToMove | Outcome::AlreadyMigrated) => {}
        Err(error) => app_log::warn(&format!(
            "daemon data not copied ({error}); {} stays in use",
            legacy.display()
        )),
    }
}

fn main() {
    let log = app_log::init(paths::log_dir());
    let source = match source_choice() {
        Ok(source) => source,
        Err(message) => {
            app_log::warn(&message);
            std::process::exit(2);
        }
    };
    app_log::info(&format!("start: {}", version::summary()));
    migrate_daemon_data();
    match &source {
        SourceChoice::Mock => app_log::info("source: demo scenarios"),
        SourceChoice::Rfd(config) => {
            app_log::info(&format!("source: daemon at {}", config.socket_path.display()));
        }
    }
    if let Some(log) = log {
        app_log::info(&format!("log: {}", log.display()));
    }
    application().with_assets(ramflux_desktop_ui::AppAssets).run(move |cx| {
        init(cx);
        ramflux_desktop_ui::init(cx);
        if let Err(err) = ramflux_desktop_ui::assets::load_fonts(cx) {
            app_log::warn(&format!("cannot load the bundled fonts: {err}"));
        }
        // Theme and language as chosen on this device (unset: the system's).
        ramflux_desktop_ui::preferences::install(AppearanceStore::default_location(), cx);
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        let bounds = Bounds::centered(None, size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(
                px(ramflux_desktop_ui::MIN_WINDOW_SIZE.0),
                px(ramflux_desktop_ui::MIN_WINDOW_SIZE.1),
            )),
            app_id: Some(identity::APP_ID.to_owned()),
            // The title compositors, docks and task switchers show (X11
            // `_NET_WM_NAME`, Wayland `xdg_toplevel.set_title`); Kit's options
            // leave it unset, which reads "Unnamed Window" on X11 window
            // managers and an empty title on Wayland.
            titlebar: Some(TitlebarOptions {
                title: Some(identity::APP_NAME.into()),
                ..TitleBar::title_bar_options()
            }),
            ..TitleBar::window_options()
        };
        let opened = open_window(options, cx, move |window, cx| {
            cx.new(|cx| Shell::application(source.clone(), window, cx))
        });
        match opened {
            Ok((window, shell)) => ramflux_desktop_ui::notify::install(window, shell, cx),
            Err(err) => {
                app_log::warn(&format!("failed to open window: {err}"));
                cx.quit();
            }
        }
    });
}
