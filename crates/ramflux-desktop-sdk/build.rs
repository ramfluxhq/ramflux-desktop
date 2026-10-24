// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Records the product commit the SDK crates are built from (as the
//! workspace lockfile resolves `ramflux-sdk`) in `RAMFLUX_PRODUCT_REV`, so
//! the version line in diagnostics can never name another revision than the
//! one compiled in.

use std::path::Path;

fn main() {
    let lock = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock.display());
    let text = std::fs::read_to_string(&lock).unwrap_or_default();
    let rev = resolved_rev(&text).unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=RAMFLUX_PRODUCT_REV={rev}");
}

/// The commit after `#` in the `ramflux-sdk` git source (the resolved
/// commit), else the `rev=` it was asked for.
fn resolved_rev(lock: &str) -> Option<String> {
    let mut in_sdk = false;
    for line in lock.lines() {
        if line.starts_with("name = ") {
            in_sdk = line == "name = \"ramflux-sdk\"";
        } else if in_sdk && let Some(source) = line.strip_prefix("source = \"git+") {
            let source = source.trim_end_matches('"');
            if let Some((_, commit)) = source.split_once('#') {
                return Some(commit.to_owned());
            }
            return source.split_once("rev=").map(|(_, rest)| {
                rest.split(['#', '"', '&']).next().unwrap_or_default().to_owned()
            });
        }
    }
    None
}
