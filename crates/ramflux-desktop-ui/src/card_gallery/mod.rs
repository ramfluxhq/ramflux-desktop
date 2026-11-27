// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The card gallery and its conformance (test builds only, feature
//! `card-gallery`; never in the default or the release build).
//!
//! The gallery lists every host fixture and scenario, draws each one through
//! the production card renderer, and runs checks A to C against the standalone
//! runtime (`ramflux-card`). `--card-gallery` on the desktop binary opens it;
//! `--card-gallery-exit` also runs the conformance once the first frame is
//! drawn, writes the records and exits with 0 when every check passes.

mod conformance;
mod fixtures;
mod view;

pub use view::{GalleryConfig, open};

/// Whether `arg` is one of the gallery's flags (the launch parser skips them).
#[must_use]
pub fn is_gallery_flag(arg: &str) -> bool {
    matches!(arg, "--card-gallery" | "--card-gallery-exit" | "--card-gallery-dark")
        || arg == "--card-gallery-locale"
}

/// Removes the gallery's flags, and the value of `--card-gallery-locale`,
/// from an argument list before the launch parser reads it.
pub fn strip_flags(args: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip_value = false;
    for arg in args {
        if skip_value {
            skip_value = false;
        } else if arg == "--card-gallery-locale" {
            skip_value = true;
        } else if !is_gallery_flag(&arg) {
            out.push(arg);
        }
    }
    out
}
