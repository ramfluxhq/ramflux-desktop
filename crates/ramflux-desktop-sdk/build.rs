// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Records the product commit the SDK crates are built from (as the
//! workspace lockfile resolves `ramflux-sdk`) in `RAMFLUX_PRODUCT_REV`, so
//! the version line in diagnostics can never name another revision than the
//! one compiled in.
//!
//! The build fails when the revision cannot be resolved. A silent fallback
//! such as "unknown" would put a wrong version line into diagnostics and
//! would hide a build script run from a stale or foreign checkout.

use std::path::Path;

fn main() {
    let lock = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock.display());
    match product_rev(&lock) {
        Ok(rev) => println!("cargo:rustc-env=RAMFLUX_PRODUCT_REV={rev}"),
        Err(reason) => {
            eprintln!("error: {reason}");
            std::process::exit(1);
        }
    }
}

/// The product revision `lock` resolves, or why it cannot be resolved.
pub fn product_rev(lock: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(lock).map_err(|error| {
        format!("cannot read {} to resolve the product revision: {error}", lock.display())
    })?;
    resolved_rev(&text).ok_or_else(|| {
        format!(
            "{} names no git source for `ramflux-sdk`; cannot resolve the product revision",
            lock.display()
        )
    })
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
