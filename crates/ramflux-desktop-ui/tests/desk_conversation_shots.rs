// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Source-backed conversation references through the production root.

#[path = "support/desk_conversation.rs"]
mod source_facts;

use gpui_kit::HeadlessAppContext;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, Point, WindowBounds, WindowOptions, px, size};
use ramflux_desktop_ui::{AppAssets, Shell};
use std::{error::Error, path::PathBuf, sync::Arc};

fn main() -> Result<(), Box<dyn Error>> {
    let out = PathBuf::from(
        std::env::var_os("RAMFLUX_SHOTS_DIR")
            .ok_or("RAMFLUX_SHOTS_DIR must name the output directory")?,
    );
    std::fs::create_dir_all(&out)?;
    for width in [640., 800., 1024., 1440.] {
        for (label, mode) in [("light", ThemeMode::Light), ("dark", ThemeMode::Dark)] {
            let mut cx = HeadlessAppContext::with_platform(
                gpui_kit::platform::current_platform(true).text_system(),
                Arc::new(AppAssets),
                gpui_kit::platform::current_headless_renderer,
            );
            cx.update(|cx| {
                gpui_kit::init(cx);
                ramflux_desktop_ui::init(cx);
                Theme::change(mode, None, cx);
                ramflux_desktop_ui::i18n::select_locale("en", cx);
            });
            let (handle, shell) = cx
                .update(|cx| {
                    gpui_kit::open_window(
                        WindowOptions {
                            window_bounds: Some(WindowBounds::Windowed(Bounds {
                                origin: Point::default(),
                                size: size(px(width), px(860.)),
                            })),
                            focus: false,
                            show: false,
                            ..Default::default()
                        },
                        cx,
                        |window, cx| {
                            cx.new(|cx| Shell::with_scenario("s02_daily", false, window, cx))
                        },
                    )
                })
                .map_err(|e| e.to_string())?;
            let store = cx.update(|cx| shell.read(cx).store().clone());
            cx.update(|cx| {
                store.update(cx, |store, cx| {
                    store.load_scenario(&source_facts::scenario(), cx);
                    store.advance(1_000, cx);
                })
            });
            cx.run_until_parked();
            let workspace = cx.update(|cx| shell.read(cx).workspace().clone());
            cx.update_window(handle, |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    workspace.select_context(
                        Some(ramflux_desktop_core::ContextId::new("c-lena")),
                        window,
                        cx,
                    )
                });
                let field = workspace.read(cx).conversation().clone();
                field.update(cx, |field, cx| {
                    field.focus_moment(&ramflux_desktop_core::MomentId::new("x-05"), cx)
                });
            })
            .map_err(|e| e.to_string())?;
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| window.render_frame(cx))
                .map_err(|e| e.to_string())?;
            let screenshot = cx.capture_screenshot(handle).map_err(|e| e.to_string())?;
            let path = out.join(format!("desk-conversation-{width}-{label}.png"));
            screenshot.save(&path)?;
            println!("wrote {}", path.display());
        }
    }
    Ok(())
}
