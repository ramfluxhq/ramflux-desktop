// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Ramflux Desktop entrypoint.

use gpui_kit::*;
use ramflux_desktop_ui::Workspace;

fn main() {
    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        if let Err(err) = open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Workspace)) {
            eprintln!("failed to open window: {err}");
            cx.quit();
        }
    });
}
