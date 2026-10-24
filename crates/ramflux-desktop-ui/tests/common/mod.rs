// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Window setup and helpers shared by the UI integration tests.

#![allow(dead_code)]

use std::time::Duration;

use gpui_kit::base::Root;
use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AppContext as _, Bounds, Entity, MouseButton, Point, TestAppContext, Window, WindowBounds,
    WindowHandle, WindowOptions, px, size,
};
use ramflux_desktop_core::{JsonValue, Store};
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::app::StoreHandle;
use ramflux_desktop_ui::confirm::HOLD_DURATION;
use ramflux_desktop_ui::route::Page;
use ramflux_desktop_ui::workspace::Workspace;

/// The window handle and the production root view.
pub type Opened = (WindowHandle<Root>, Entity<Shell>);

/// How long a condition-based wait on real threads or processes may take
/// before the test fails. It only bounds a failure; no test asserts speed.
#[allow(dead_code)]
pub const WAIT_LIMIT: Duration = Duration::from_secs(30);

/// Opens the production window root playing `scenario` with a running clock.
pub fn open(cx: &mut TestAppContext, scenario: &str) -> Opened {
    cx.update(|cx| {
        gpui_kit::init(cx);
        ramflux_desktop_ui::init(cx);
    });
    let name = scenario.to_owned();
    let (handle, shell) = cx.update(|cx| {
        let bounds = Bounds { origin: Point::default(), size: size(px(1280.), px(860.)) };
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

/// Opens the production window root set up by `options`.
pub fn open_with(
    cx: &mut TestAppContext,
    options: ramflux_desktop_ui::shell::ShellOptions,
) -> Opened {
    open_sized(cx, options, 860.)
}

/// Opens the production window, `height` pixels tall (for pages whose
/// lower groups a test operates).
pub fn open_sized(
    cx: &mut TestAppContext,
    options: ramflux_desktop_ui::shell::ShellOptions,
    height: f32,
) -> Opened {
    cx.update(|cx| {
        gpui_kit::init(cx);
        ramflux_desktop_ui::init(cx);
    });
    let (handle, shell) = cx.update(|cx| {
        let bounds = Bounds { origin: Point::default(), size: size(px(1280.), px(height)) };
        let (window, shell) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            move |window, cx| cx.new(|cx| Shell::with_options(options, window, cx)),
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
    let result = cx
        .update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            f(window, cx)
        })
        .expect("window");
    cx.run_until_parked();
    result
}

/// Clicks an element by id.
pub fn click(cx: &mut TestAppContext, handle: WindowHandle<Root>, id: &'static str) {
    in_window(cx, handle, |window, cx| window.click(id, cx));
}

/// Clicks an element by id inside the element `scope`.
pub fn click_in(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    scope: &str,
    id: &'static str,
) {
    let scope = scope.to_owned();
    in_window(cx, handle, move |window, cx| window.within(scope).click(id, cx));
}

/// Focuses an input by id and types `text`.
pub fn type_into(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    id: &'static str,
    text: &str,
) {
    let text = text.to_owned();
    in_window(cx, handle, move |window, cx| {
        window.click(id, cx);
        window.input(&text, cx);
    });
}

/// Whether an element with `id` was painted in the last frame.
pub fn exists(cx: &mut TestAppContext, handle: WindowHandle<Root>, id: &str) -> bool {
    let id = id.to_owned();
    in_window(cx, handle, move |window, _| window.try_find(id).is_some())
}

/// Whether an element labeled `label` was painted in the last frame.
pub fn has_label(cx: &mut TestAppContext, handle: WindowHandle<Root>, label: &str) -> bool {
    let label = label.to_owned();
    in_window(cx, handle, move |window, _| !window.find_all_by_label(&label).is_empty())
}

/// Holds the confirm control for longer than [`HOLD_DURATION`].
pub fn hold_confirm(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    let center = in_window(cx, handle, |window, cx| {
        let center = window.find("hold-confirm").bounds().center();
        window.pointer_down(center, MouseButton::Left, cx);
        center
    });
    advance(cx, u64::try_from(HOLD_DURATION.as_millis()).unwrap_or(0) + 200);
    in_window(cx, handle, |window, cx| window.pointer_up(center, MouseButton::Left, cx));
}

pub fn store_handle(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Entity<StoreHandle> {
    cx.update(|cx| shell.read(cx).store().clone())
}

pub fn workspace(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Entity<Workspace> {
    cx.update(|cx| shell.read(cx).workspace().clone())
}

pub fn page(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Page {
    cx.update(|cx| shell.read(cx).page())
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
    bodies(cx, shell).into_iter().map(|(label, _)| label).collect()
}

/// Every effect the active source received, with its secret-free body.
pub fn bodies(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Vec<(String, JsonValue)> {
    cx.update(|cx| {
        shell
            .read(cx)
            .store()
            .read(cx)
            .session()
            .map(|s| s.effects().map(|(l, b)| (l.to_owned(), b.clone())).collect())
            .unwrap_or_default()
    })
}

/// The body of the last effect with `method`.
pub fn last_body(cx: &mut TestAppContext, shell: &Entity<Shell>, method: &str) -> JsonValue {
    bodies(cx, shell)
        .into_iter()
        .rev()
        .find(|(label, _)| label == method)
        .map(|(_, body)| body)
        .unwrap_or_else(|| panic!("no {method} effect"))
}

/// Whether the hold-to-confirm strip is open.
pub fn strip_open(cx: &mut TestAppContext, shell: &Entity<Shell>) -> bool {
    cx.update(|cx| shell.read(cx).root().read(cx).confirm_strip().read(cx).is_open())
}

/// Ids of the rows of the shown conversation.
pub fn rows(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Vec<String> {
    let workspace = workspace(cx, shell);
    cx.update(|cx| {
        workspace
            .read(cx)
            .conversation()
            .read(cx)
            .row_ids()
            .into_iter()
            .map(|id| id.to_string())
            .collect()
    })
}

/// Shows a context through the workspace navigation API.
pub fn show_context(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    shell: &Entity<Shell>,
    id: &str,
) {
    let workspace = workspace(cx, shell);
    let id = ramflux_desktop_core::ContextId::new(id);
    cx.update_window(handle.into(), |_, window, cx| {
        workspace.update(cx, |w, cx| w.select_context(Some(id), window, cx));
    })
    .expect("window");
    cx.run_until_parked();
}

/// Whether an element with `id` inside the element `scope` was painted.
pub fn exists_in(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    scope: &str,
    id: &str,
) -> bool {
    let (scope, id) = (scope.to_owned(), id.to_owned());
    in_window(cx, handle, move |window, _| window.within(scope).try_find(id).is_some())
}
