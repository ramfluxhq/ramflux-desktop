// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The product revision shown in diagnostics is the one the lockfile pins.

#[test]
fn the_shown_product_revision_is_the_pinned_one() {
    let pinned = ramflux_desktop_test::live_daemon::pinned_product_rev().expect("pinned revision");
    assert_eq!(
        ramflux_desktop_sdk::PRODUCT_REV,
        pinned,
        "PRODUCT_REV is read from the lockfile at build time"
    );
}

/// The build script itself, to check how it fails (its `main` is unused here).
#[allow(dead_code, clippy::print_stderr)]
#[path = "../build.rs"]
mod build_script;

#[test]
fn a_lockfile_that_cannot_be_read_stops_the_build_instead_of_saying_unknown() {
    let missing = std::env::temp_dir().join("ramflux-no-such-dir-desk-pin-03").join("Cargo.lock");
    let error = build_script::product_rev(&missing).expect_err("no lockfile");
    assert!(error.contains("cannot read"), "{error}");
    assert!(error.contains("Cargo.lock"), "{error}");
    assert!(!error.contains("unknown"), "{error}");
}

#[test]
fn a_lockfile_without_the_product_source_stops_the_build() {
    let dir = std::env::temp_dir().join(format!("ramflux-lock-desk-pin-03-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let lock = dir.join("Cargo.lock");
    std::fs::write(&lock, "[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n").expect("write");
    let error = build_script::product_rev(&lock).expect_err("no product source");
    assert!(error.contains("names no git source"), "{error}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_real_lockfile_resolves_to_the_pinned_revision() {
    let lock = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock");
    let pinned = ramflux_desktop_test::live_daemon::pinned_product_rev().expect("pinned revision");
    assert_eq!(build_script::product_rev(&lock).as_deref(), Ok(pinned.as_str()));
}
