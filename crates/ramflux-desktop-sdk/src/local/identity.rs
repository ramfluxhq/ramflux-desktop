// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The application's identity on the desktop: one id for every place an
//! operating system or desktop environment recognises the app.
//!
//! [`APP_ID`] is the Wayland `app_id` and X11 `WM_CLASS` of the window, the
//! basename of the Linux desktop entry, the hicolor icon name, the AppStream
//! component id, the `desktop-entry` hint of notifications and the macOS
//! bundle identifier (`scripts/package-macos.sh`). A test keeps the
//! packaging files in step with it.

/// Reverse-DNS application id.
pub const APP_ID: &str = "org.ramflux.desktop";

/// The name people see (launcher entry, notification sender).
pub const APP_NAME: &str = "Ramflux";

/// The folder name under the XDG base directories (Linux and other Unix).
pub const XDG_DIR_NAME: &str = "ramflux-desktop";

/// The folder name under the macOS and Windows per-user locations.
pub const NATIVE_DIR_NAME: &str = "Ramflux Desktop";

/// The executable's file name.
pub const EXECUTABLE: &str = "ramflux-desktop";

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn repo_file(path: &str) -> String {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        std::fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    #[test]
    fn packaging_files_use_the_one_app_id() {
        let entry = repo_file(&format!("packaging/linux/{APP_ID}.desktop"));
        for line in [
            format!("Icon={APP_ID}"),
            format!("StartupWMClass={APP_ID}"),
            format!("Exec={EXECUTABLE}"),
            format!("Name={APP_NAME}"),
        ] {
            assert!(entry.lines().any(|l| l == line), "desktop entry lacks {line}");
        }
        let metainfo = repo_file(&format!("packaging/linux/{APP_ID}.metainfo.xml"));
        assert!(metainfo.contains(&format!("<id>{APP_ID}</id>")), "metainfo id");
        assert!(
            metainfo.contains(&format!(
                "<launchable type=\"desktop-id\">{APP_ID}.desktop</launchable>"
            )),
            "metainfo launchable"
        );
        assert!(
            repo_file("scripts/package-macos.sh").contains(&format!("BUNDLE_ID=\"{APP_ID}\"")),
            "macOS bundle id"
        );
        assert!(
            repo_file("scripts/package-linux.sh").contains(&format!("APP_ID=\"{APP_ID}\"")),
            "Linux package id"
        );
        let icons = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packaging/linux/icons");
        for size in ["48x48", "64x64", "128x128", "256x256", "512x512"] {
            let icon = icons.join(format!("hicolor/{size}/apps/{APP_ID}.png"));
            assert!(icon.is_file(), "missing {}", icon.display());
        }
        assert!(icons.join(format!("hicolor/scalable/apps/{APP_ID}.svg")).is_file());
    }
}
