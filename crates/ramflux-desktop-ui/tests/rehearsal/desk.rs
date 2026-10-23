// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! One desktop: the production window root on its own daemon, driven
//! through its painted controls (click, type, hold) and read back through
//! its store. Every helper returns an error instead of panicking, so a
//! failed step is recorded and the rehearsal goes on.

use std::path::Path;
use std::time::{Duration, Instant};

use gpui_kit::base::Root;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
use gpui_kit::{
    AppContext as _, Bounds, Entity, MouseButton, Point, WindowBounds, WindowHandle, WindowOptions,
    px, size,
};
use ramflux_desktop_core::{ContextId, Scope, Store, UiEventKind};
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::confirm::HOLD_DURATION;
use ramflux_desktop_ui::root::IntentRoot;
use ramflux_desktop_ui::route::Page;
use ramflux_desktop_ui::shell::ShellOptions;

use super::host::Host;

/// One desktop window.
pub struct Desk {
    /// Who this desktop belongs to in the rehearsal ("a", "b").
    pub tag: &'static str,
    handle: WindowHandle<Root>,
    shell: Entity<Shell>,
}

impl Desk {
    /// Opens the production window root set up by `options`, offscreen.
    pub fn open(host: &mut impl Host, tag: &'static str, options: ShellOptions) -> Self {
        let (handle, shell) = host.app(|cx| {
            let bounds = Bounds { origin: Point::default(), size: size(px(1280.), px(860.)) };
            let (window, shell) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                cx,
                move |window, cx| cx.new(|cx| Shell::with_options(options, window, cx)),
            )
            .expect("offscreen window");
            (window.downcast::<Root>().expect("Root"), shell)
        });
        host.advance(10);
        Self { tag, handle, shell }
    }

    /// Reads this desktop's active store.
    pub fn store<R>(&self, host: &mut impl Host, f: impl FnOnce(&Store) -> R) -> R {
        host.app(|cx| f(self.shell.read(cx).store().read(cx).store()))
    }

    /// The page shown.
    pub fn page(&self, host: &mut impl Host) -> Page {
        host.app(|cx| self.shell.read(cx).page())
    }

    /// Whether an element with `id` was painted.
    pub fn exists(&self, host: &mut impl Host, id: &str) -> bool {
        let id = id.to_owned();
        host.window(self.handle.into(), move |window, _| window.try_find(id).is_some())
            .unwrap_or(false)
    }

    /// Whether an element with `id` inside the element `scope` was painted.
    pub fn exists_in(&self, host: &mut impl Host, scope: &str, id: &str) -> bool {
        let (scope, id) = (scope.to_owned(), id.to_owned());
        host.window(self.handle.into(), move |window, _| {
            window.within(scope).try_find(id).is_some()
        })
        .unwrap_or(false)
    }

    /// The accessible label of the element `id`, if painted.
    pub fn label(&self, host: &mut impl Host, id: &str) -> Option<String> {
        let id = id.to_owned();
        host.window(self.handle.into(), move |window, _| {
            window.try_find(id).and_then(|e| e.label().map(str::to_owned))
        })
        .ok()
        .flatten()
    }

    /// Clicks the element `id` (inside the element `scope`, when given).
    pub fn click(&self, host: &mut impl Host, scope: Option<&str>, id: &str) -> Result<(), String> {
        let (scope, id) = (scope.map(str::to_owned), id.to_owned());
        let found = host.window(self.handle.into(), move |window, cx| {
            let present = match &scope {
                Some(scope) => window.within(scope.clone()).try_find(id.clone()).is_some(),
                None => window.try_find(id.clone()).is_some(),
            };
            if present {
                match scope {
                    Some(scope) => window.within(scope).click(id, cx),
                    None => window.click(id, cx),
                }
            }
            present
        })?;
        if found { Ok(()) } else { Err(format!("desktop {}: no control to click", self.tag)) }
    }

    /// Focuses the input `id` and types `text`.
    pub fn type_into(&self, host: &mut impl Host, id: &str, text: &str) -> Result<(), String> {
        let (id, text) = (id.to_owned(), text.to_owned());
        let found = host.window(self.handle.into(), move |window, cx| {
            if window.try_find(id.clone()).is_none() {
                return false;
            }
            window.click(id, cx);
            window.input(&text, cx);
            true
        })?;
        if found { Ok(()) } else { Err(format!("desktop {}: no input to type into", self.tag)) }
    }

    /// Whether the hold-to-confirm strip is open (it is never a modal).
    pub fn strip_open(&self, host: &mut impl Host) -> bool {
        host.app(|cx| self.shell.read(cx).root().read(cx).confirm_strip().read(cx).is_open())
    }

    /// Holds the confirm control for longer than the hold duration.
    pub fn hold_confirm(&self, host: &mut impl Host) -> Result<(), String> {
        let center = host.window(self.handle.into(), |window, cx| {
            let center = window.try_find("hold-confirm")?.bounds().center();
            window.pointer_down(center, MouseButton::Left, cx);
            Some(center)
        })?;
        let center = center.ok_or_else(|| format!("desktop {}: no hold control", self.tag))?;
        host.advance(u64::try_from(HOLD_DURATION.as_millis()).unwrap_or(0) + 200);
        host.window(self.handle.into(), move |window, cx| {
            window.pointer_up(center, MouseButton::Left, cx);
        })
    }

    /// Shows a conversation through the workspace navigation.
    pub fn show_context(&self, host: &mut impl Host, id: &ContextId) -> Result<(), String> {
        let workspace = host.app(|cx| self.shell.read(cx).workspace().clone());
        let id = id.clone();
        host.window(self.handle.into(), move |window, cx| {
            workspace.update(cx, |w, cx| w.select_context(Some(id), window, cx));
        })
    }

    /// Emits a UI event from inside conversation `context`, through the
    /// root sink every view emits through (UiEvent, then root, then intent).
    pub fn emit_in(
        &self,
        host: &mut impl Host,
        context: &ContextId,
        kind: UiEventKind,
    ) -> Result<(), String> {
        let root = host.app(|cx| self.shell.read(cx).root().clone());
        let scope = Scope::Context(context.clone());
        host.window(self.handle.into(), move |window, cx| {
            IntentRoot::sink(&root).scoped(scope).emit(kind, window, cx);
        })
    }

    /// Opens Settings at `section`.
    pub fn open_settings(
        &self,
        host: &mut impl Host,
        section: ramflux_desktop_ui::settings::SettingsSection,
    ) {
        host.app(|cx| self.shell.update(cx, |shell, cx| shell.open_settings_at(section, cx)));
        host.advance(10);
    }

    /// Closes Settings when open.
    pub fn close_settings(&self, host: &mut impl Host) {
        host.app(|cx| {
            self.shell.update(cx, |shell, cx| {
                if shell.is_settings_open() {
                    shell.close_settings_now(cx);
                }
            });
        });
        host.advance(10);
    }

    /// Ticks until `done` holds or `limit` passes.
    pub fn until<H: Host>(
        &self,
        host: &mut H,
        limit: Duration,
        what: &str,
        mut done: impl FnMut(&Self, &mut H) -> bool,
    ) -> Result<Duration, String> {
        let start = Instant::now();
        while !done(self, host) {
            if start.elapsed() > limit {
                return Err(format!(
                    "desktop {}: timed out after {limit:?} waiting for {what}",
                    self.tag
                ));
            }
            host.tick();
        }
        Ok(start.elapsed())
    }

    /// Saves light and dark renders as `<name>_<tag>_<mode>.png` in `dir`
    /// with the developer panel closed. Returns how many files were written
    /// (none on a context without a renderer).
    pub fn shots(&self, host: &mut impl Host, dir: &Path, name: &str) -> Result<usize, String> {
        if host.app(|cx| self.shell.read(cx).is_dev_panel_visible()) {
            return Err("screenshots are taken with the developer panel closed".to_owned());
        }
        let mut written = 0;
        for (mode, suffix) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
            host.app(|cx| {
                Theme::change(mode, None, cx);
                cx.refresh_windows();
            });
            host.window(self.handle.into(), |_, _| {})?;
            let path = dir.join(format!("{name}_{}_{suffix}.png", self.tag));
            if host.screenshot(self.handle.into(), &path)? {
                written += 1;
            }
        }
        host.app(|cx| {
            Theme::change(ThemeMode::Light, None, cx);
            cx.refresh_windows();
        });
        host.advance(10);
        Ok(written)
    }
}
