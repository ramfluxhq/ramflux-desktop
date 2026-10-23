// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Window setup shared by the UI integration tests.

#![allow(dead_code)]

use std::time::Duration;

use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Entity, Point, TestAppContext, Window, WindowBounds, WindowHandle,
    WindowOptions, px, size,
};
use ramflux_desktop_core::Store;
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::app::StoreHandle;
use ramflux_desktop_ui::workspace::Workspace;

/// Opens the production window root playing `scenario` with a running clock.
pub fn open(cx: &mut TestAppContext, scenario: &str) -> (WindowHandle<Root>, Entity<Shell>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        ramflux_desktop_ui::init(cx);
    });
    let name = scenario.to_owned();
    let (handle, shell) = cx.update(|cx| {
        let bounds = Bounds { origin: Point::default(), size: size(px(1280.), px(820.)) };
        let (window, shell) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            move |window, cx| cx.new(|cx| Shell::with_scenario(&name, true, window, cx)),
        )
        .expect("open window");
        (window.downcast::<Root>().expect("Root"), shell)
    });
    cx.run_until_parked();
    (handle, shell)
}

/// Moves the GPUI test clock (and so the scenario clock) forward.
pub fn advance(cx: &mut TestAppContext, ms: u64) {
    cx.executor().advance_clock(Duration::from_millis(ms));
    cx.run_until_parked();
}

/// Runs `f` inside the window with a fresh frame.
pub fn in_window<R>(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    f: impl FnOnce(&mut Window, &mut gpui_kit::App) -> R,
) -> R {
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        f(window, cx)
    })
    .expect("window")
}

pub fn store_handle(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Entity<StoreHandle> {
    cx.update(|cx| shell.read(cx).store().clone())
}

pub fn workspace(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Entity<Workspace> {
    cx.update(|cx| shell.read(cx).workspace().clone())
}

/// Reads the active store.
pub fn with_store<R>(
    cx: &mut TestAppContext,
    shell: &Entity<Shell>,
    f: impl FnOnce(&Store) -> R,
) -> R {
    cx.update(|cx| f(shell.read(cx).store().read(cx).store()))
}

/// Labels of every effect the active source received.
pub fn effects(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Vec<String> {
    cx.update(|cx| {
        shell
            .read(cx)
            .store()
            .read(cx)
            .session()
            .map(|s| s.effect_labels().map(str::to_owned).collect())
            .unwrap_or_default()
    })
}
