// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Keyboard commands and their bindings.
//!
//! Commands dispatch along the focus path and bubble outward; a handler that
//! cannot act calls `cx.propagate()` so an outer level can.

use gpui_kit::{App, KeyBinding};

gpui_kit::actions!(
    ramflux,
    [
        /// Show or hide the developer panel (debug builds).
        ToggleDevPanel,
        /// Focus the oldest moment that needs the user.
        GoToNeedsMe,
        /// Pin or unpin the current context.
        TogglePinContext,
        /// Open settings.
        OpenSettings,
        /// Close settings.
        CloseSettings,
        /// Switch between the light and dark theme.
        ToggleTheme,
        /// Cancel the pending hold-to-confirm request.
        CancelConfirmation,
        /// Open the command palette.
        OpenPalette,
        /// Close the command palette.
        ClosePalette,
    ]
);

/// Key context of the confirmation strip.
pub const CONFIRM_CONTEXT: &str = "ConfirmStrip";
/// Key context of the settings page.
pub const SETTINGS_CONTEXT: &str = "Settings";
/// Key context of the command palette overlay.
pub const PALETTE_CONTEXT: &str = "Palette";

/// Registers the application key bindings. Call before building menus.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-j", GoToNeedsMe, None),
        KeyBinding::new("secondary-shift-p", TogglePinContext, None),
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-shift-t", ToggleTheme, None),
        KeyBinding::new("secondary-k", OpenPalette, None),
        KeyBinding::new("escape", ClosePalette, Some(PALETTE_CONTEXT)),
        KeyBinding::new("escape", CancelConfirmation, Some(CONFIRM_CONTEXT)),
        KeyBinding::new("escape", CloseSettings, Some(SETTINGS_CONTEXT)),
    ]);
    if cfg!(debug_assertions) {
        cx.bind_keys([KeyBinding::new("secondary-shift-d", ToggleDevPanel, None)]);
    }
}
