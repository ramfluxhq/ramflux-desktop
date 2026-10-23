// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The GPUI context the rehearsal drives: the offscreen headless context of
//! the live run (real renderer, screenshots) or the test context of the
//! fake-daemon tests (no renderer). Both move the GPUI clock while real time
//! passes for the daemons and the bridge threads.

use std::path::Path;
use std::time::Duration;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, App, AppContext as _, HeadlessAppContext, TestAppContext, Window};

/// What the rehearsal needs from a GPUI context.
pub trait Host {
    /// Runs `f` on the application.
    fn app<R>(&mut self, f: impl FnOnce(&mut App) -> R) -> R;

    /// Runs `f` in `window` after rendering a fresh frame, then lets pending
    /// work settle.
    fn window<R>(
        &mut self,
        window: AnyWindowHandle,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Result<R, String>;

    /// Moves the GPUI clock forward by `ms` and runs what became due.
    fn advance(&mut self, ms: u64);

    /// Saves a render of `window` to `path`. `Ok(false)` when this context
    /// has no renderer.
    fn screenshot(&mut self, window: AnyWindowHandle, path: &Path) -> Result<bool, String>;

    /// Lets real time pass for threads and child processes, then moves the
    /// GPUI clock.
    fn tick(&mut self) {
        std::thread::sleep(Duration::from_millis(10));
        self.advance(50);
    }
}

impl Host for HeadlessAppContext {
    fn app<R>(&mut self, f: impl FnOnce(&mut App) -> R) -> R {
        self.update(f)
    }

    fn window<R>(
        &mut self,
        window: AnyWindowHandle,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Result<R, String> {
        let result = self
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                f(window, cx)
            })
            .map_err(|e| format!("window: {e}"))?;
        self.run_until_parked();
        Ok(result)
    }

    fn advance(&mut self, ms: u64) {
        self.advance_clock(Duration::from_millis(ms));
        self.run_until_parked();
    }

    fn screenshot(&mut self, window: AnyWindowHandle, path: &Path) -> Result<bool, String> {
        let image = self.capture_screenshot(window).map_err(|e| format!("render: {e}"))?;
        image.save(path).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        Ok(true)
    }
}

impl Host for TestAppContext {
    fn app<R>(&mut self, f: impl FnOnce(&mut App) -> R) -> R {
        self.update(f)
    }

    fn window<R>(
        &mut self,
        window: AnyWindowHandle,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Result<R, String> {
        let result = self
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                f(window, cx)
            })
            .map_err(|e| format!("window: {e}"))?;
        self.run_until_parked();
        Ok(result)
    }

    fn advance(&mut self, ms: u64) {
        self.executor().advance_clock(Duration::from_millis(ms));
        self.run_until_parked();
    }

    fn screenshot(&mut self, _: AnyWindowHandle, _: &Path) -> Result<bool, String> {
        Ok(false)
    }
}
