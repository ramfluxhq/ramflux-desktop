// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Ramflux Desktop entrypoint.
//!
//! ```text
//! ramflux-desktop [--source mock|rfd] [--socket PATH]
//! ```
//!
//! The source defaults to the bundled scenarios in debug builds and to the
//! local daemon in release builds. `RAMFLUX_DESKTOP_SOURCE` and
//! `RAMFLUX_DESKTOP_SOCKET` set the same choices from the environment; flags
//! win.

use gpui_kit::component::TitleBar;
use gpui_kit::*;
use ramflux_desktop_sdk::SdkConfig;
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::shell::SourceChoice;

/// Initial window size: a physical window boundary, not application layout.
const WINDOW_SIZE: (f32, f32) = (1280., 820.);

/// The source chosen by flags, then the environment, then the build.
fn source_choice() -> Result<SourceChoice, String> {
    let mut source = std::env::var("RAMFLUX_DESKTOP_SOURCE").ok();
    let mut socket = std::env::var("RAMFLUX_DESKTOP_SOCKET").ok();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--source" => source = args.next(),
            "--socket" => socket = args.next(),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    let rfd = || SourceChoice::Rfd(socket.clone().map_or_else(SdkConfig::default, SdkConfig::new));
    match source.as_deref() {
        None if socket.is_some() => Ok(rfd()),
        None => Ok(SourceChoice::default_for_build()),
        Some("mock") => Ok(SourceChoice::Mock),
        Some("rfd") => Ok(rfd()),
        Some(other) => Err(format!("unknown source: {other} (use mock or rfd)")),
    }
}

fn main() {
    let source = match source_choice() {
        Ok(source) => source,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    application().with_assets(assets::Assets).run(move |cx| {
        init(cx);
        ramflux_desktop_ui::init(cx);
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        let bounds = Bounds::centered(None, size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..TitleBar::window_options()
        };
        let opened = open_window(options, cx, move |window, cx| {
            cx.new(|cx| Shell::application(source.clone(), window, cx))
        });
        if let Err(err) = opened {
            eprintln!("failed to open window: {err}");
            cx.quit();
        }
    });
}
