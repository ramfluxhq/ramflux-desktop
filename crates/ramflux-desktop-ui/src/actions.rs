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
        /// Show the first context of the stack and focus the composer.
        SelectStackEntry1,
        /// Show the second context of the stack.
        SelectStackEntry2,
        /// Show the third context of the stack.
        SelectStackEntry3,
        /// Show the fourth context of the stack.
        SelectStackEntry4,
        /// Show the fifth context of the stack.
        SelectStackEntry5,
        /// Show the sixth context of the stack.
        SelectStackEntry6,
        /// Open or close the objects dock.
        ToggleObjectsDock,
        /// Open or close the contacts dock.
        ToggleContactsDock,
        /// Open or close the members dock.
        ToggleMembersDock,
        /// Open or close the approvals dock.
        ToggleApprovalsDock,
        /// Open or close the evidence dock.
        ToggleEvidenceDock,
        /// Close the dock that has focus and return to the composer.
        CloseDock,
        /// Show the next settings section.
        NextSettingsSection,
        /// Show the previous settings section.
        PreviousSettingsSection,
    ]
);

/// Key context of the confirmation strip.
pub const CONFIRM_CONTEXT: &str = "ConfirmStrip";
/// Key context of the settings page.
pub const SETTINGS_CONTEXT: &str = "Settings";
/// Key context of the command palette overlay.
pub const PALETTE_CONTEXT: &str = "Palette";
/// Key context of a dock tool panel.
pub const DOCK_CONTEXT: &str = "DockTool";

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
        KeyBinding::new("secondary-1", SelectStackEntry1, None),
        KeyBinding::new("secondary-2", SelectStackEntry2, None),
        KeyBinding::new("secondary-3", SelectStackEntry3, None),
        KeyBinding::new("secondary-4", SelectStackEntry4, None),
        KeyBinding::new("secondary-5", SelectStackEntry5, None),
        KeyBinding::new("secondary-6", SelectStackEntry6, None),
        KeyBinding::new("secondary-alt-1", ToggleObjectsDock, None),
        KeyBinding::new("secondary-alt-2", ToggleContactsDock, None),
        KeyBinding::new("secondary-alt-3", ToggleMembersDock, None),
        KeyBinding::new("secondary-alt-4", ToggleApprovalsDock, None),
        KeyBinding::new("secondary-alt-5", ToggleEvidenceDock, None),
        KeyBinding::new("escape", CloseDock, Some(DOCK_CONTEXT)),
        KeyBinding::new("ctrl-tab", NextSettingsSection, Some(SETTINGS_CONTEXT)),
        KeyBinding::new("ctrl-shift-tab", PreviousSettingsSection, Some(SETTINGS_CONTEXT)),
    ]);
    if cfg!(debug_assertions) {
        cx.bind_keys([KeyBinding::new("secondary-shift-d", ToggleDevPanel, None)]);
    }
}
