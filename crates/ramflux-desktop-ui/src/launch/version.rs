// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Version information shown in Settings › Diagnostics and written to the
//! application log at start.

/// The desktop's version (the workspace version).
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The desktop's source revision, when the build recorded one (the macOS
/// packaging script sets `RAMFLUX_DESKTOP_BUILD_REV`).
pub const BUILD_REV: Option<&str> = option_env!("RAMFLUX_DESKTOP_BUILD_REV");

/// The GPUI Kit release the workspace pins.
pub const GPUI_KIT_VERSION: &str = "0.8.0";

/// The product revision the SDK crates are pinned to.
pub const PRODUCT_REV: &str = ramflux_desktop_sdk::PRODUCT_REV;

/// The first seven characters of a revision.
pub fn short(rev: &str) -> &str {
    rev.get(..7).unwrap_or(rev)
}

/// One line: `Ramflux Desktop 0.1.0 (build abc1234), product fd3b184,
/// GPUI Kit 0.8.0`.
pub fn summary() -> String {
    let build = BUILD_REV.map_or_else(String::new, |rev| format!(" (build {})", short(rev)));
    format!(
        "Ramflux Desktop {APP_VERSION}{build}, product {}, GPUI Kit {GPUI_KIT_VERSION}",
        short(PRODUCT_REV)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gpui_kit_version_matches_the_workspace_pin() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
        let text = std::fs::read_to_string(manifest).expect("workspace manifest");
        assert!(
            text.contains(&format!("gpui-kit = \"={GPUI_KIT_VERSION}\"")),
            "update GPUI_KIT_VERSION with the gpui-kit pin"
        );
        assert!(summary().contains(APP_VERSION));
        assert_eq!(short("fd3b184bec025f45"), "fd3b184");
        assert_eq!(short("abc"), "abc");
    }
}
