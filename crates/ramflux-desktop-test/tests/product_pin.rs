// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The product pin: the three product revisions in the workspace manifest are
//! one and the same commit, and the lock file moved only as far as that pin
//! requires.
//!
//! Lock scope, by class: (a) the product crates take the pinned commit;
//! (b) packages newly required by the pinned product manifests are the listed
//! `ALLOWED_NEW` ones; (c) no existing package changes its full identity (name, version, source,
//! checksum); (d) dependency edge rewires are allowed because only package
//! identities are compared. The
//! baseline fixture lists every non-product package of the lock before the
//! pin moved; a later repin adds its new packages to a list of their own.

use std::collections::BTreeSet;
use std::path::PathBuf;

const PINNED_REV: &str = "b847923b1e064ddd6e993dbc625da62f3cd30f08";
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
    "pbjson|0.9.0|registry+https://github.com/rust-lang/crates.io-index|e8edd1efdd8ab23ba9cb9ace3d9987a72663d5d7c9f74fa00b51d6213645cf6c",
    "prost|0.14.4|registry+https://github.com/rust-lang/crates.io-index|528ac67416ff8646872a3c02cad9cc4ee5dc9f9540c9b10771855c95cb2e5ae1",
    "prost-derive|0.14.4|registry+https://github.com/rust-lang/crates.io-index|b570b25f7617e43d59005d0990ccb79e950a423952cea19671b7a876da390adf",
    "prost-reflect|0.16.5|registry+https://github.com/rust-lang/crates.io-index|01b80ea363c31af2de2b92e3c07ed1156628f7838c4afb4df75ee78a37fedbd1",
    "prost-types|0.14.4|registry+https://github.com/rust-lang/crates.io-index|f94967dc7688f3054c7fac87473ffae4cc4c3904800e2d9f5b857246d8963b0a",
];
/// Packages the pin to `b847923b` (VAULT P07b1) newly requires: the Linux typed
/// vault providers' dependency closure (`keyring`, `secret-service`, `dbus`
/// and `openssl`, with older `zbus`/`zvariant`/`windows-sys`/`foreign-types`
/// versions living beside the ones already locked). No existing version is
/// bumped or dropped.
const ALLOWED_NEW_VAULT: [&str; 21] = [
    "dbus|0.9.12|registry+https://github.com/rust-lang/crates.io-index|3ab69f03cc8c4340c9c8e315114e1658e6775a9b16a04357973aa21cec22b32e",
    "dbus-secret-service|4.1.0|registry+https://github.com/rust-lang/crates.io-index|708b509edf7889e53d7efb0ffadd994cc6c2345ccb62f55cfd6b0682165e4fa6",
    "foreign-types|0.3.2|registry+https://github.com/rust-lang/crates.io-index|f6f339eb8adc052cd2ca78910fda869aefa38d22d5cb648e6485e4d3fc06f3b1",
    "foreign-types-shared|0.1.1|registry+https://github.com/rust-lang/crates.io-index|00b0228411908ca8685dba7fc2cdd70ec9990a6e753e89b6ac91a84c40fbaf4b",
    "keyring|3.6.3|registry+https://github.com/rust-lang/crates.io-index|eebcc3aff044e5944a8fbaf69eb277d11986064cba30c468730e8b9909fb551c",
    "libdbus-sys|0.2.7|registry+https://github.com/rust-lang/crates.io-index|328c4789d42200f1eeec05bd86c9c13c7f091d2ba9a6ea35acdf51f31bc0f043",
    "nix|0.29.0|registry+https://github.com/rust-lang/crates.io-index|71e2746dc3a24dd78b3cfcb7be93368c6de9963d30f43a6a73998a9cf4b17b46",
    "openssl|0.10.82|registry+https://github.com/rust-lang/crates.io-index|4de8111e94a56adca7ebfe6c365b6d42f99deb69b74b6becacd4204e2f193aa2",
    "openssl-macros|0.1.1|registry+https://github.com/rust-lang/crates.io-index|a948666b637a0f465e8564c73e89d4dde00d72d4d473cc972f390fc3dcee7d9c",
    "openssl-src|300.6.1+3.6.3|registry+https://github.com/rust-lang/crates.io-index|46eb8fb9fb3b61ce1c0f8a026c4c1a0714d3a9e138e7fbde78753ce2babc3846",
    "openssl-sys|0.9.118|registry+https://github.com/rust-lang/crates.io-index|19c413d6ec9ebaf5e1c829d355cb3e9f783778d700f135be0921d52fce3c60ae",
    "secret-service|4.0.0|registry+https://github.com/rust-lang/crates.io-index|e4d35ad99a181be0a60ffcbe85d680d98f87bdc4d7644ade319b87076b9dbfd4",
    "sha1|0.10.7|registry+https://github.com/rust-lang/crates.io-index|a978451301f4db1d02937a4ab3ccce137717b81826e79b7d49ffe3244a13c3b8",
    "windows-sys|0.59.0|registry+https://github.com/rust-lang/crates.io-index|1e38bc4d79ed67fd075bcc251a1c39b32a1776bbe92e5bef1f0bf1f8c531853b",
    "xdg-home|1.3.0|registry+https://github.com/rust-lang/crates.io-index|ec1cdab258fb55c0da61328dc52c8764709b249011b2cad0454c72f0bf10a1f6",
    "zbus|4.4.0|registry+https://github.com/rust-lang/crates.io-index|bb97012beadd29e654708a0fdb4c84bc046f537aecfde2c3ee0a9e4b4d48c725",
    "zbus_macros|4.4.0|registry+https://github.com/rust-lang/crates.io-index|267db9407081e90bbfa46d841d3cbc60f59c0351838c4bc65199ecd79ab1983e",
    "zbus_names|3.0.0|registry+https://github.com/rust-lang/crates.io-index|4b9b1fef7d021261cc16cba64c351d291b715febe0fa10dc3a443ac5a5022e6c",
    "zvariant|4.2.0|registry+https://github.com/rust-lang/crates.io-index|2084290ab9a1c471c38fc524945837734fbf124487e105daec2bb57fd48c81fe",
    "zvariant_derive|4.2.0|registry+https://github.com/rust-lang/crates.io-index|73e2ba546bda683a90652bac4a279bc146adad1386f25379cf73200d2002c449",
    "zvariant_utils|2.1.0|registry+https://github.com/rust-lang/crates.io-index|c51bcff7cc3dbb5055396bcf774748c3dab426b4b8659046963523cee4808340",
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

/// `(name, version, source, checksum)` of every package in the lock file.
fn lock_packages(lock: &str) -> Vec<(String, String, String, String)> {
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
            (field("name"), field("version"), field("source"), field("checksum"))
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
    for (name, _, source, _) in &packages {
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
        .filter(|(name, _, source, _)| {
            !name.starts_with("ramflux-")
                || !source.is_empty() && !source.starts_with(PRODUCT_SOURCE)
        })
        .map(|(name, version, source, checksum)| format!("{name}|{version}|{source}|{checksum}"))
        .collect();
    let mut allowed: BTreeSet<String> = BASELINE.lines().map(str::to_owned).collect();
    allowed.extend(ALLOWED_NEW.iter().map(|s| (*s).to_owned()));
    allowed.extend(ALLOWED_NEW_VAULT.iter().map(|s| (*s).to_owned()));
    let unexpected: Vec<&String> = actual.difference(&allowed).collect();
    let missing: Vec<&String> = allowed.difference(&actual).collect();
    assert!(
        unexpected.is_empty(),
        "lock has unclassified packages, version bumps or source/checksum changes: {unexpected:?}"
    );
    assert!(missing.is_empty(), "lock dropped packages: {missing:?}");
}
