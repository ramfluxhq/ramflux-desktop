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
//! local daemon at the reference CLI's socket in release builds.
//!
//! The desktop attaches to a daemon that is already running. It never
//! starts one by itself; the entry page offers "Start local daemon".
//! Application events go to the platform log directory, redacted.

use gpui_kit::component::TitleBar;
use gpui_kit::*;
use ramflux_desktop_sdk::local::{LaunchStore, app_log, paths};
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
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        let bounds = Bounds::centered(None, size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(
                px(ramflux_desktop_ui::MIN_WINDOW_SIZE.0),
                px(ramflux_desktop_ui::MIN_WINDOW_SIZE.1),
            )),
            ..TitleBar::window_options()
        };
        let opened = open_window(options, cx, move |window, cx| {
            cx.new(|cx| Shell::application(source.clone(), window, cx))
        });
        if let Err(err) = opened {
            app_log::warn(&format!("failed to open window: {err}"));
            cx.quit();
        }
    });
}
