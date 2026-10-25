// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Locale resource checks: every key has every locale, English text has no
//! CJK characters, every key used in code exists, keys are unique across the
//! per-area files, the runtime resolves every file, and no Rust source in the
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

fn locale_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(crate_dir().join("locales"))
        .expect("locales dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "yml"))
        .collect();
    files.sort();
    files
}

/// Parses every two-space-indented `_version: 2` file into key -> locale -> text.
fn parse_locales() -> BTreeMap<String, BTreeMap<String, String>> {
    let mut entries: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for file in locale_files() {
        for (key, texts) in parse_file(&file) {
            assert!(!entries.contains_key(&key), "{key} is defined twice ({})", file.display());
            entries.insert(key, texts);
        }
    }
    entries
}

fn parse_file(file: &Path) -> BTreeMap<String, BTreeMap<String, String>> {
    let text = fs::read_to_string(file).expect("locale file");
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
    assert!(missing.is_empty(), "keys used in code but missing in the locale files: {missing:?}");
}

#[test]
fn the_runtime_resolves_a_key_from_every_file() {
    rust_i18n::set_locale("en");
    for file in locale_files() {
        let entries = parse_file(&file);
        for (key, texts) in entries.iter().filter(|(_, t)| !t["en"].contains('%')).take(3) {
            let text = ramflux_desktop_ui::i18n::tr(key);
            assert_eq!(
                text.as_ref(),
                texts["en"].as_str(),
                "{key} from {} is not loaded",
                file.display()
            );
        }
    }
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

#[test]
fn every_reason_the_sdk_source_writes_is_translated() {
    let entries = parse_locales();
    let reasons = ramflux_desktop_sdk::capabilities::unsupported()
        .into_values()
        .chain(ramflux_desktop_sdk::translate::reason::key::ALL)
        .chain([
            ramflux_desktop_sdk::bus::TIMEOUT_KEY,
            ramflux_desktop_sdk::bus::RECONCILE_KEY,
            ramflux_desktop_sdk::bus::UNAUTHENTICATED_KEY,
            ramflux_desktop_sdk::bus::TRANSPORT_SECURITY_KEY,
        ]);
    for key in reasons {
        assert!(entries.contains_key(key), "{key} has no translation");
    }
}

#[test]
fn a_reason_with_arguments_is_translated_with_them() {
    rust_i18n::set_locale("en");
    let text = ramflux_desktop_ui::i18n::backend_text(
        r#"backend.failure.partial {"done":"1","total":"2","reason":"backend.failure.timeout"}"#,
    );
    let timeout = ramflux_desktop_ui::i18n::tr("backend.failure.timeout");
    assert_eq!(text.as_ref(), format!("Stopped after 1 of 2: {timeout}"));
    let daemon = ramflux_desktop_ui::i18n::backend_text("local bus error: x");
    assert_eq!(daemon.as_ref(), "local bus error: x", "daemon text is shown as written");
    let odd = ramflux_desktop_ui::i18n::backend_text("backend.failure.partial not-json");
    assert_eq!(odd.as_ref(), "backend.failure.partial not-json", "never a half translation");
}

/// User-facing text names no internal work items (card ids such as `K8`)
/// and no protocol terms; those belong in docs and the dev panel's keys.
#[test]
fn user_facing_text_has_no_card_ids_or_protocol_terms() {
    let card_id = |text: &str| {
        let chars: Vec<char> = text.chars().collect();
        chars
            .windows(3)
            .any(|w| matches!(w[0], '(' | '\u{FF08}') && w[1] == 'K' && w[2].is_ascii_digit())
    };
    const PROTOCOL_TERMS: [&str; 3] = ["A2UI", "CAS ", "principal id"];
    for (key, texts) in parse_locales() {
        for (locale, text) in &texts {
            assert!(!card_id(text), "{key} ({locale}) names an internal card id: {text}");
            for term in PROTOCOL_TERMS {
                assert!(
                    !text.to_lowercase().contains(&term.to_lowercase()),
                    "{key} ({locale}) uses the protocol term {term:?}: {text}"
                );
            }
        }
    }
}
