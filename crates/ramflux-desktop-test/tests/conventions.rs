// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The lint wiring that enforces the workspace conventions cannot be left
//! out by a crate: every member opts in to the workspace lints, and every
//! crate root denies unwrap, expect, panic and `std::sync` locks in its
//! production code (tests may use them).

use std::path::{Path, PathBuf};

/// The attribute every crate root carries, as rustfmt lays it out.
const PRODUCTION_DENY: &str = "#![cfg_attr(
    not(test),
    deny(clippy::disallowed_types, clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The `members` of the workspace manifest.
fn members() -> Vec<String> {
    let manifest = read(&root().join("Cargo.toml"));
    let start = manifest.find("members = [").expect("workspace members");
    let list = &manifest[start..];
    let end = list.find(']').expect("members end");
    list[..end]
        .lines()
        .skip(1)
        .filter_map(|line| line.trim().strip_prefix('"')?.strip_suffix("\",").map(str::to_owned))
        .collect()
}

#[test]
fn every_member_takes_the_workspace_lints_and_denies_them_in_production_code() {
    let members = members();
    assert!(members.len() >= 7, "members read from the manifest: {members:?}");
    for member in members {
        let dir = root().join(&member);
        let manifest = read(&dir.join("Cargo.toml"));
        assert!(
            manifest.contains("[lints]\nworkspace = true\n"),
            "{member} does not opt in to the workspace lints"
        );
        let lib = dir.join("src/lib.rs");
        let crate_root = if lib.exists() { lib } else { dir.join("src/main.rs") };
        assert!(
            read(&crate_root).contains(PRODUCTION_DENY),
            "{} does not deny unwrap, expect, panic and std locks in production code",
            crate_root.display()
        );
    }
}

#[test]
fn the_std_locks_are_the_disallowed_types() {
    let clippy = read(&root().join("clippy.toml"));
    for lock in ["std::sync::Mutex", "std::sync::RwLock", "std::sync::Condvar"] {
        assert!(clippy.contains(&format!("path = \"{lock}\"")), "{lock} is not disallowed");
    }
    let manifest = read(&root().join("Cargo.toml"));
    for lint in ["unsafe_code = \"forbid\"", "dbg_macro = \"deny\"", "todo = \"deny\""] {
        assert!(manifest.contains(lint), "workspace lint missing: {lint}");
    }
}
