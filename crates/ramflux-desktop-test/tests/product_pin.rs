// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The product pin: the three product revisions in the workspace manifest are
//! one and the same commit, and the lock file moved only as far as that pin
//! requires.
//!
//! Lock scope, by class: (a) the product crates take the pinned commit;
//! (b) packages newly required by the pinned product manifests are the listed
//! `ALLOWED_NEW` ones; (c) no existing package changes version; (d) dependency
//! edge rewires are allowed because only package versions are compared. The
//! baseline fixture lists every non-product package of the lock before the
//! pin moved; a later repin updates the fixture and `ALLOWED_NEW` with it.

use std::collections::BTreeSet;
use std::path::PathBuf;

const PINNED_REV: &str = "77f76871ca357056ac88e5c0997cff9026abd2e4";
const PRODUCT_SOURCE: &str = "git+https://github.com/ramfluxhq/ramflux.git";
const PRODUCT_CRATES: [&str; 7] = [
    "ramflux-core",
    "ramflux-crypto",
    "ramflux-protocol",
    "ramflux-sdk",
    "ramflux-storage",
    "ramflux-sync",
    "ramflux-transport",
];
const ALLOWED_NEW: [&str; 5] = [
    "pbjson 0.9.0",
    "prost 0.14.4",
    "prost-derive 0.14.4",
    "prost-reflect 0.16.5",
    "prost-types 0.14.4",
];
const BASELINE: &str = include_str!("fixtures/product_pin_lock_baseline.txt");

fn root_file(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The `rev = "..."` values of the product dependencies in the manifest.
fn manifest_revs(manifest: &str) -> Vec<String> {
    manifest
        .lines()
        .filter(|line| line.contains("ramfluxhq/ramflux.git"))
        .map(|line| {
            let rest = line.split("rev = \"").nth(1).expect("product line has a rev");
            rest.split('"').next().expect("rev is quoted").to_owned()
        })
        .collect()
}

/// `(name, version, source)` of every package in the lock file.
fn lock_packages(lock: &str) -> Vec<(String, String, String)> {
    lock.split("[[package]]")
        .skip(1)
        .map(|block| {
            let field = |key: &str| {
                block
                    .lines()
                    .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix(" = \"")))
                    .map(|r| r.trim_end_matches('"').to_owned())
                    .unwrap_or_default()
            };
            (field("name"), field("version"), field("source"))
        })
        .collect()
}

#[test]
fn product_pin_single_rev_and_lock_scope() {
    let revs = manifest_revs(&root_file("Cargo.toml"));
    assert_eq!(revs.len(), 3, "sdk, protocol and crypto are pinned");
    assert!(revs.iter().all(|r| r == PINNED_REV), "all product revs equal the pin: {revs:?}");

    let packages = lock_packages(&root_file("Cargo.lock"));

    // (a) every product package comes from the pinned commit, and no other.
    let mut product_seen = BTreeSet::new();
    for (name, _, source) in &packages {
        if source.starts_with(PRODUCT_SOURCE) {
            assert!(
                source.ends_with(&format!("#{PINNED_REV}"))
                    && source.contains(&format!("rev={PINNED_REV}")),
                "{name} is not at the pinned commit: {source}"
            );
            product_seen.insert(name.as_str());
        }
    }
    let expected: BTreeSet<&str> = PRODUCT_CRATES.into_iter().collect();
    assert_eq!(product_seen, expected, "product packages in the lock");

    // (b), (c): every other registry package is the baseline plus the
    // allowed new ones, so a version bump or an unexpected package fails.
    let actual: BTreeSet<String> = packages
        .iter()
        .filter(|(name, _, source)| {
            !name.starts_with("ramflux-")
                || !source.is_empty() && !source.starts_with(PRODUCT_SOURCE)
        })
        .map(|(name, version, _)| format!("{name} {version}"))
        .collect();
    let mut allowed: BTreeSet<String> = BASELINE.lines().map(str::to_owned).collect();
    allowed.extend(ALLOWED_NEW.iter().map(|s| (*s).to_owned()));
    let unexpected: Vec<&String> = actual.difference(&allowed).collect();
    let missing: Vec<&String> = allowed.difference(&actual).collect();
    assert!(
        unexpected.is_empty(),
        "lock has unclassified packages or version bumps: {unexpected:?}"
    );
    assert!(missing.is_empty(), "lock dropped packages: {missing:?}");
}
