// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Lists the theme token sets the design compiler exported to
//! `design/generated/` (every `*.json` there except the manifest) and embeds
//! them, so a new theme is a new token file: no code names a theme.
//!
//! The build fails when the directory cannot be read or holds no theme.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The import manifest, which is not a theme.
const MANIFEST: &str = "manifest.json";

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../design/generated");
    println!("cargo:rerun-if-changed={}", dir.display());
    let Some(out) = std::env::var_os("OUT_DIR") else {
        eprintln!("error: OUT_DIR is not set");
        std::process::exit(1);
    };
    match theme_files(&dir).and_then(|files| render(&files)) {
        Ok(source) => {
            if let Err(error) = std::fs::write(PathBuf::from(out).join("themes.rs"), source) {
                eprintln!("error: cannot write the theme list: {error}");
                std::process::exit(1);
            }
        }
        Err(reason) => {
            eprintln!("error: {reason}");
            std::process::exit(1);
        }
    }
}

/// The token files in `dir`, as (id, path), sorted by id.
fn theme_files(dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|error| format!("cannot read the theme directory {}: {error}", dir.display()))?;
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let is_json = path.extension().is_some_and(|ext| ext == "json");
        let is_manifest = path.file_name().is_some_and(|name| name == MANIFEST);
        if !is_json || is_manifest {
            continue;
        }
        println!("cargo:rerun-if-changed={}", path.display());
        let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            return Err(format!("theme file name is not UTF-8: {}", path.display()));
        };
        files.push((id.to_owned(), path.canonicalize().map_err(|e| e.to_string())?));
    }
    files.sort();
    if files.is_empty() {
        return Err(format!("no theme token set in {}", dir.display()));
    }
    Ok(files)
}

/// The embedded list as Rust source.
fn render(files: &[(String, PathBuf)]) -> Result<String, String> {
    let mut source = String::from("/// Every exported theme token set: (id, JSON).\n");
    source.push_str("pub(crate) const THEME_FILES: &[(&str, &str)] = &[\n");
    for (id, path) in files {
        let Some(path) = path.to_str() else {
            return Err(format!("theme path is not UTF-8: {}", path.display()));
        };
        writeln!(source, "    ({id:?}, include_str!({path:?})),").map_err(|e| e.to_string())?;
    }
    source.push_str("];\n");
    Ok(source)
}
