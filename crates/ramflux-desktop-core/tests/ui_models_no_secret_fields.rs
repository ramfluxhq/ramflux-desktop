// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Static guard (S10c D3): no secret field enters the core or UI models.
//!
//! The core models and the UI read only what the daemon shows; key slots,
//! encrypted keys, read capabilities, group key material, decision-proof
//! secrets, raw inner bytes and unsupported-content raw handles stay in the
//! SDK. The scan covers every identifier in the model sources.

use std::path::{Path, PathBuf};

/// Identifiers no model may declare or read.
const FORBIDDEN: [&str; 11] = [
    "key_slots",
    "encrypted_key",
    "read_capability",
    "raw_handle",
    "group_key",
    "key_distribution",
    "distribution_key",
    "decision_proof",
    "proof_secret",
    "raw_inner",
    "inner_bytes",
];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("source dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn offending(files: &[PathBuf], forbidden: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(file).expect("source");
        for (line, content) in text.lines().enumerate() {
            let lower = content.to_ascii_lowercase();
            for word in forbidden {
                if lower.contains(word) {
                    found.push(format!("{}:{}: {word}", file.display(), line + 1));
                }
            }
        }
    }
    found
}

#[test]
fn ui_models_have_no_secret_fields() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    sources(&crates.join("ramflux-desktop-core/src"), &mut files);
    sources(&crates.join("ramflux-desktop-ui/src"), &mut files);
    // The v1 projection reads typed input and emits only models.
    sources(&crates.join("ramflux-desktop-sdk/src/translate/v1"), &mut files);
    assert!(files.len() > 50, "the scan sees the sources");
    assert_eq!(offending(&files, &FORBIDDEN), Vec::<String>::new());
}

#[test]
fn the_guard_catches_a_secret_field() {
    let dir = std::env::temp_dir().join(format!("rf-guard-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("model.rs");
    std::fs::write(&file, "pub struct Slot {\n    pub key_slots: Vec<u8>,\n}\n").expect("write");
    let found = offending(&[file], &FORBIDDEN);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(found.len(), 1);
    assert!(found[0].ends_with(":2: key_slots"), "{found:?}");
}
