// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Ramflux Desktop entrypoint.

use gpui_kit::component::TitleBar;
use gpui_kit::*;
use ramflux_desktop_ui::Shell;

/// Initial window size: a physical window boundary, not application layout.
const WINDOW_SIZE: (f32, f32) = (1280., 820.);

fn main() {
    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        ramflux_desktop_ui::init(cx);
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        let bounds = Bounds::centered(None, size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..TitleBar::window_options()
        };
        if let Err(err) = open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(window, cx)))
        {
            eprintln!("failed to open window: {err}");
            cx.quit();
        }
    });
}
