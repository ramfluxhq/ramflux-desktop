// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Whether the window draws its own title bar (Linux and Windows; macOS has
//! its own drag region, see `shell.rs`).
//!
//! On Linux the window asks for server-side decorations (GPUI's default when
//! no decoration mode is set). Where the compositor grants them (Hyprland,
//! KDE, Sway; Omarchy's floating workspaces draw a themed title bar) it
//! draws the title bar and buttons itself, and a second row of our own would
//! be wasted space. Where it does not (GNOME), GPUI falls back to
//! client-side decorations and reports the tiling state; a window tiled on
//! all four edges by a tiling layout has no use for a draggable bar either.
//! A floating or maximized client-decorated window, or one snapped to half
//! the screen, keeps the bar: it is the only way to move it and the only
//! close button.
//!
//! GPUI Kit's title bar already omits its minimize, maximize and close
//! buttons under server-side decorations and shows only the controls the
//! compositor supports; this decides about the bar as a whole.

use gpui_kit::{Decorations, Tiling, Window};

/// Whether a window with `decorations` and `maximized` draws the title bar,
/// on Linux (`linux`) or another non-macOS platform.
pub fn title_bar_for(linux: bool, decorations: Decorations, maximized: bool) -> bool {
    if !linux {
        // Windows: Kit's title bar carries the window controls.
        return true;
    }
    match decorations {
        Decorations::Server => false,
        Decorations::Client { tiling } => maximized || tiling != Tiling::tiled(),
    }
}

/// Whether `window` draws the title bar now.
pub fn draws_title_bar(window: &Window) -> bool {
    title_bar_for(cfg!(target_os = "linux"), window.window_decorations(), window.is_maximized())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(top: bool, left: bool, right: bool, bottom: bool) -> Decorations {
        Decorations::Client { tiling: Tiling { top, left, right, bottom } }
    }

    #[test]
    fn server_decorations_hide_the_bar_on_linux() {
        assert!(!title_bar_for(true, Decorations::Server, false));
        assert!(!title_bar_for(true, Decorations::Server, true));
    }

    #[test]
    fn a_fully_tiled_client_window_hides_the_bar() {
        assert!(!title_bar_for(true, client(true, true, true, true), false));
    }

    #[test]
    fn floating_maximized_and_half_tiled_windows_keep_the_bar() {
        assert!(title_bar_for(true, client(false, false, false, false), false), "floating");
        assert!(title_bar_for(true, client(true, true, true, true), true), "maximized");
        assert!(title_bar_for(true, client(true, true, false, true), false), "snapped left");
    }

    #[test]
    fn other_platforms_always_draw_it() {
        assert!(title_bar_for(false, Decorations::Server, false));
        assert!(title_bar_for(false, client(true, true, true, true), false));
    }
}
