// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Locale resource checks: every key has every locale, `en` and `zh-CN`
//! have exactly the same keys with no empty text, English text has no CJK
//! characters, every key used in code exists, plural messages carry every
//! category, keys are unique across the per-area files, the runtime resolves
//! every file, no Rust source in the workspace contains CJK text (translated
//! text lives only in locale files), and no view passes a hard-coded
//! user-visible string to a text, label, tooltip or accessibility API.

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
    // A plural message is used by its base key and defined per category.
    let missing: Vec<&String> = used
        .iter()
        .filter(|key| !entries.contains_key(*key) && !entries.contains_key(&format!("{key}.other")))
        .collect();
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

/// Every `en` and `zh-CN` leaf of a file, by key, read strictly: an empty
/// text is kept (as empty) instead of being taken for a group.
fn strict_leaves(file: &Path) -> BTreeMap<&'static str, BTreeMap<String, String>> {
    let text = fs::read_to_string(file).expect("test: locale file reads");
    let mut path: Vec<String> = Vec::new();
    let mut leaves: BTreeMap<&'static str, BTreeMap<String, String>> = BTreeMap::new();
    for line in text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with("_version")) {
        let depth = (line.len() - line.trim_start().len()) / 2;
        let (name, value) = line.trim().split_once(':').expect("test: key: value");
        path.truncate(depth);
        if let Some(locale) = LOCALES.iter().find(|l| **l == name) {
            let value = value.trim().trim_matches('"').to_owned();
            leaves.entry(locale).or_default().insert(path.join("."), value);
        } else {
            path.push(name.to_owned());
        }
    }
    leaves
}

#[test]
fn en_and_zh_cn_have_exactly_the_same_keys_and_no_empty_text() {
    let mut total = 0;
    for file in locale_files() {
        let leaves = strict_leaves(&file);
        let keys = |locale: &str| -> BTreeSet<String> {
            leaves.get(locale).map(|m| m.keys().cloned().collect()).unwrap_or_default()
        };
        let (en, zh) = (keys("en"), keys("zh-CN"));
        let only_en: Vec<_> = en.difference(&zh).collect();
        let only_zh: Vec<_> = zh.difference(&en).collect();
        assert!(only_en.is_empty(), "{}: only in en: {only_en:?}", file.display());
        assert!(only_zh.is_empty(), "{}: only in zh-CN: {only_zh:?}", file.display());
        for (locale, texts) in &leaves {
            for (key, text) in texts {
                assert!(!text.trim().is_empty(), "{key} ({locale}) has empty text");
            }
        }
        total += en.len();
    }
    assert!(total > 1_000, "parsed {total} keys per locale");
}

#[test]
fn plural_messages_carry_one_and_other_with_the_same_placeholders() {
    let entries = parse_locales();
    let placeholders = |text: &str| -> BTreeSet<String> {
        text.split("%{")
            .skip(1)
            .filter_map(|p| p.split_once('}'))
            .map(|(n, _)| n.to_owned())
            .collect()
    };
    let mut plurals = 0;
    for (key, texts) in &entries {
        let Some(base) = key.strip_suffix(".one") else { continue };
        plurals += 1;
        let other = entries
            .get(&format!("{base}.other"))
            .unwrap_or_else(|| panic!("{base} has `one` but no `other`"));
        for locale in LOCALES {
            assert_eq!(
                placeholders(&texts[locale]),
                placeholders(&other[locale]),
                "{base} ({locale}): `one` and `other` must take the same arguments"
            );
        }
    }
    assert!(plurals >= 20, "found {plurals} plural messages");
}

#[test]
fn a_plural_message_follows_the_count_and_formats_it() {
    use ramflux_desktop_ui::i18n::{self, tr_args, tr_count};
    rust_i18n::set_locale("en");
    assert_eq!(tr_count("palette.matches", 1).as_ref(), "1 match");
    assert_eq!(tr_count("palette.matches", 3).as_ref(), "3 matches");
    assert_eq!(tr_count("palette.matches", 12_345).as_ref(), "12,345 matches");
    assert_eq!(
        tr_args("moment.task.count", &[("done", "1"), ("total", "1")]).as_ref(),
        "1 of 1 step",
        "without a count the total picks the form"
    );
    assert_eq!(i18n::bytes(482_133), "470.8 KiB");
    assert_eq!(i18n::bytes(512), "512 B");
    assert_eq!(i18n::percent(0.425).as_ref(), "43%");
    assert_eq!(i18n::join("a", "b").as_ref(), "a \u{b7} b");
    assert_eq!(i18n::sentences("Offline", "Retry later").as_ref(), "Offline. Retry later");
}

#[test]
fn date_and_time_patterns_are_valid_in_every_locale() {
    let entries = parse_locales();
    for key in ["format.time", "format.date", "format.date_time"] {
        for locale in LOCALES {
            let pattern = &entries[key][locale];
            let items = chrono::format::StrftimeItems::new(pattern);
            assert!(
                !items.into_iter().any(|item| matches!(item, chrono::format::Item::Error)),
                "{key} ({locale}) is not a valid date pattern: {pattern}"
            );
        }
    }
}

/// Methods and constructors that put text on screen or into the
/// accessibility tree.
const TEXT_APIS: [&str; 16] = [
    ".child(",
    ".aria_label(",
    ".accessibility_label(",
    ".tooltip(",
    ".label(",
    ".placeholder(",
    ".title(",
    ".description(",
    ".prompt(",
    ".message(",
    "Label::new(",
    "PopupMenuItem::new(",
    "PopupMenuItem::label(",
    "Tooltip::new(",
    "Notification::new(",
    "SharedString::new_static(",
];

/// Decorative glyphs that are the same in every language (separator dots,
/// a mask bullet, an ellipsis).
const GLYPHS: [&str; 6] = ["\\u{b7}", "\\u{2022}", "\\u{2026}", "\u{b7}", "\u{2022}", "\u{2026}"];

/// The production part of a source file (before its unit tests).
fn production(text: &str) -> &str {
    text.find("#[cfg(test)]").map_or(text, |end| &text[..end])
}

/// The string literal at the start of `rest` (after optional whitespace and
/// an optional `format!(`), if any.
fn leading_literal(rest: &str) -> Option<&str> {
    let rest = rest.trim_start();
    let rest = rest.strip_prefix("format!(").map_or(rest, str::trim_start);
    let body = rest.strip_prefix('"')?;
    let mut escaped = false;
    for (i, c) in body.char_indices() {
        match c {
            '\\' if !escaped => escaped = true,
            '"' if !escaped => return Some(&body[..i]),
            _ => escaped = false,
        }
    }
    None
}

/// What a literal shows besides placeholders, whitespace and decorative
/// glyphs.
fn visible_text(literal: &str) -> String {
    let mut shown = String::new();
    let mut depth = 0;
    for c in literal.chars() {
        match c {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            _ if depth == 0 => shown.push(c),
            _ => {}
        }
    }
    for glyph in GLYPHS {
        shown = shown.replace(glyph, "");
    }
    shown.chars().filter(|c| !c.is_whitespace()).collect()
}

#[test]
fn no_view_passes_a_hard_coded_user_visible_string() {
    // The scanner itself: words are caught, placeholders and glyphs are not.
    assert_eq!(leading_literal(r#" format!("Save {name}")"#), Some("Save {name}"));
    assert_eq!(visible_text("Save {name}"), "Save");
    assert_eq!(visible_text("{a} \u{b7} {b}"), "");
    let mut files = Vec::new();
    rust_files(&crate_dir().join("src"), &mut files);
    rust_files(&crate_dir().join("../../apps"), &mut files);
    let mut found = Vec::new();
    for file in files {
        let source = fs::read_to_string(&file).expect("test: source reads");
        let code = production(&source);
        for api in TEXT_APIS {
            for (at, _) in code.match_indices(api) {
                let Some(literal) = leading_literal(&code[at + api.len()..]) else { continue };
                let shown = visible_text(literal);
                if !shown.is_empty() {
                    let line = code[..at].lines().count();
                    found.push(format!("{}:{line}: {api}\"{literal}\"", file.display()));
                }
            }
        }
        // Lists are joined with the locale's separator.
        for (at, _) in code.match_indices(".join(") {
            // Punctuation differs per language; spaces, newlines and path
            // segments do not.
            let punctuated = leading_literal(&code[at + ".join(".len()..])
                .is_some_and(|sep| sep.contains([',', ';', '\u{b7}']) || sep.contains("\\u{b7}"));
            let path = file.display().to_string();
            let exempt = ["/dev/", "/diagnostics.rs", "/root/mod.rs", "/launch/"];
            if punctuated && !exempt.iter().any(|dir| path.contains(dir)) {
                let line = code[..at].lines().count();
                found.push(format!("{path}:{line}: joins a list with a fixed separator"));
            }
        }
        // Separators joining translated parts belong to the locale too.
        for literal in code.split('"').skip(1).step_by(2) {
            let joins = literal.contains('{')
                && (literal.contains('\u{b7}') || literal.contains("\\u{b7}"));
            if joins {
                found.push(format!(
                    "{}: joins with a fixed separator: \"{literal}\"",
                    file.display()
                ));
            }
        }
    }
    assert!(
        found.is_empty(),
        "hard-coded user-visible text (use the locale files):\n{}",
        found.join("\n")
    );
}
