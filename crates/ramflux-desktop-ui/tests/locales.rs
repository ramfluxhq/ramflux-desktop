// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Locale resource checks: every key has every locale, English text has no
//! CJK characters, every key used in code exists, and no Rust source in the
//! workspace contains CJK text (translated text lives only in locale files).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const LOCALES: [&str; 2] = ["en", "zh-CN"];

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3000..=0x303F | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
}

/// Parses the two-space-indented `_version: 2` file into key -> locale -> text.
fn parse_locales() -> BTreeMap<String, BTreeMap<String, String>> {
    let text = fs::read_to_string(crate_dir().join("locales/ui.yml")).expect("locale file");
    let mut path: Vec<String> = Vec::new();
    let mut entries: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for line in text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with("_version")) {
        let indent = line.len() - line.trim_start().len();
        assert_eq!(indent % 2, 0, "odd indentation: {line}");
        let depth = indent / 2;
        let (name, value) = line.trim().split_once(':').expect("key: value");
        path.truncate(depth);
        let value = value.trim().trim_matches('"').to_owned();
        if LOCALES.contains(&name) && !value.is_empty() {
            entries.entry(path.join(".")).or_default().insert(name.to_owned(), value);
        } else {
            assert!(value.is_empty(), "a group key must not carry text: {line}");
            path.push(name.to_owned());
        }
    }
    entries
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Every string literal shaped like a translation key whose first segment is
/// a top-level group of the locale file (this also catches keys selected in
/// `match` arms before being passed to `tr`).
fn used_keys(groups: &BTreeSet<String>) -> BTreeSet<String> {
    let mut files = Vec::new();
    rust_files(&crate_dir().join("src"), &mut files);
    let is_key = |s: &str| {
        s.contains('.')
            && s.split('.').all(|part| {
                !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase() || c == '_')
            })
    };
    let mut keys = BTreeSet::new();
    for file in files {
        let text = fs::read_to_string(&file).expect("source");
        for literal in text.split('"').skip(1).step_by(2) {
            let group = literal.split('.').next().unwrap_or_default();
            if is_key(literal) && groups.contains(group) {
                keys.insert(literal.to_owned());
            }
        }
    }
    keys
}

#[test]
fn every_key_has_every_locale_and_english_has_no_cjk() {
    let entries = parse_locales();
    assert!(entries.len() > 100, "parsed {} keys", entries.len());
    for (key, texts) in &entries {
        for locale in LOCALES {
            assert!(texts.contains_key(locale), "{key} is missing {locale}");
        }
        let english = &texts["en"];
        let allowed_native_name = key == "locale.chinese";
        assert!(
            allowed_native_name || !english.chars().any(is_cjk),
            "English text of {key} contains CJK: {english}"
        );
        assert!(!english.contains("..."), "{key} uses three dots instead of an ellipsis");
    }
}

#[test]
fn every_key_used_in_code_exists() {
    let entries = parse_locales();
    let groups: BTreeSet<String> =
        entries.keys().filter_map(|k| k.split('.').next()).map(str::to_owned).collect();
    let used = used_keys(&groups);
    assert!(used.len() > 100, "found {} keys in code", used.len());
    let missing: Vec<&String> = used.iter().filter(|key| !entries.contains_key(*key)).collect();
    assert!(missing.is_empty(), "keys used in code but missing in ui.yml: {missing:?}");
}

#[test]
fn no_rust_source_contains_cjk_text() {
    let workspace = crate_dir().join("../..");
    let mut files = Vec::new();
    for dir in ["crates", "apps"] {
        rust_files(&workspace.join(dir), &mut files);
    }
    assert!(!files.is_empty());
    for file in files {
        let text = fs::read_to_string(&file).expect("source");
        assert!(!text.chars().any(is_cjk), "{} contains CJK text", file.display());
    }
}
