// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Records the source revision as `RAMFLUX_DESKTOP_BUILD_REV` when the build environment
//! (the macOS packaging script) did not already set it, so every build reports its commit.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=RAMFLUX_DESKTOP_BUILD_REV");
    if std::env::var_os("RAMFLUX_DESKTOP_BUILD_REV").is_some() {
        return;
    }
    let Some(rev) = git(&["rev-parse", "HEAD"]) else { return };
    println!("cargo:rustc-env=RAMFLUX_DESKTOP_BUILD_REV={rev}");
    if let Some(git_dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        println!("cargo:rerun-if-changed={git_dir}/HEAD");
    }
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!text.is_empty()).then_some(text)
}
