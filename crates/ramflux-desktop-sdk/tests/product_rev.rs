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
