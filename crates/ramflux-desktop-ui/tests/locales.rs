// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Catalogue checks, run by `cargo test`:
//!
//! - every shipped locale has exactly one catalogue (`locales/<tag>.json`),
//!   in canonical form, with exactly the source key set: the English keys,
//!   each plural message carrying the categories of the locale's language
//!   (an untranslated entry is `null`);
//! - English and Simplified Chinese are complete;
//! - every key used in code exists, and every key is used (literally, or
//!   under a prefix the code builds keys from);
//! - placeholders agree between every locale and English;
//! - date patterns are valid, English text has no CJK, user-facing text
//!   names no internal card ids or protocol terms;
//! - no Rust source contains CJK text and no view passes a hard-coded
//!   user-visible string to a text, label, tooltip or accessibility API;
//! - the coverage report (`docs/i18n-coverage.md`) is current.
//!
//! `RAMFLUX_LOCALES_SYNC=1 cargo test -p ramflux-desktop-ui --test locales --
//! --test-threads=1` rewrites every catalogue in canonical form (adding `null` for new keys,
//! dropping removed ones, creating the catalogue of a newly shipped locale)
//! and regenerates the coverage report.
//!
//! Every test here that looks text up uses the source locale only, so the
//! tests do not race on the process-wide locale.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use ramflux_desktop_ui::i18n::locale::{SHIPPED, SOURCE};
use ramflux_desktop_ui::i18n::plural::{self, Category};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn syncing() -> bool {
    std::env::var_os("RAMFLUX_LOCALES_SYNC").is_some()
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3000..=0x303F | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
}

/// One catalogue file, in file order: key and text (`None`: untranslated).
type Entries = Vec<(String, Option<String>)>;

/// Reads a catalogue in canonical form: an opening brace, one
/// `  "key": value` line per entry (a comma after all but the last), a
/// closing brace. Panics on anything else, or a duplicate key.
fn read_catalogue(path: &Path) -> Entries {
    let text = fs::read_to_string(path).expect("test: catalogue reads");
    let lines: Vec<&str> = text.lines().collect();
    assert!(text.ends_with("}\n"), "{}: ends with a closing brace and a newline", path.display());
    assert_eq!(lines.first(), Some(&"{"), "{}: starts with a brace", path.display());
    assert_eq!(lines.last(), Some(&"}"), "{}: ends with a brace", path.display());
    let body = &lines[1..lines.len() - 1];
    let mut seen = BTreeSet::new();
    let mut entries = Vec::new();
    for (i, line) in body.iter().enumerate() {
        let last = i + 1 == body.len();
        let line = if last {
            *line
        } else {
            line.strip_suffix(',')
                .unwrap_or_else(|| panic!("{}: `{line}` needs a comma", path.display()))
        };
        let entry = line
            .strip_prefix("  ")
            .unwrap_or_else(|| panic!("{}: indent `{line}`", path.display()));
        let pair: serde_json::Value = serde_json::from_str(&format!("{{{entry}}}"))
            .unwrap_or_else(|e| panic!("{}: `{line}`: {e}", path.display()));
        let (key, value) = pair
            .as_object()
            .and_then(|o| o.iter().next())
            .map(|(k, v)| (k.clone(), v.clone()))
            .unwrap_or_else(|| panic!("{}: `{line}` is one entry", path.display()));
        assert!(seen.insert(key.clone()), "{}: {key} is defined twice", path.display());
        let text = match value {
            serde_json::Value::String(text) => Some(text),
            serde_json::Value::Null => None,
            other => panic!("{}: {key} is {other}, not a string or null", path.display()),
        };
        entries.push((key, text));
    }
    entries
}

fn catalogue_path(tag: &str) -> PathBuf {
    crate_dir().join("locales").join(format!("{tag}.json"))
}

/// The catalogue of every file in `locales/`, by tag.
fn catalogues() -> BTreeMap<String, Entries> {
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(crate_dir().join("locales")).expect("test: locales dir").flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            let tag = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_owned();
            out.insert(tag, read_catalogue(&path));
        }
    }
    out
}

fn source() -> Entries {
    read_catalogue(&catalogue_path(SOURCE))
}

/// The plural messages of the source catalogue: a base whose children are
/// all plural categories, `other` among them.
fn plural_bases(source: &Entries) -> BTreeSet<String> {
    let keys: BTreeSet<&str> = source.iter().map(|(k, _)| k.as_str()).collect();
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for key in &keys {
        if let Some((base, last)) = key.rsplit_once('.') {
            children.entry(base).or_default().push(last);
        }
    }
    children
        .into_iter()
        .filter(|(base, names)| {
            names.contains(&"other")
                && names.iter().all(|n| Category::parse(n).is_some())
                && !keys.contains(base)
                && !keys.iter().any(|k| {
                    k.starts_with(&format!("{base}.")) && k[base.len() + 1..].contains('.')
                })
        })
        .map(|(base, _)| base.to_owned())
        .collect()
}

/// The keys catalogue `tag` must have, in order: the source keys, with each
/// plural message's categories replaced by those of `tag`'s language.
fn expected_keys(tag: &str, source: &Entries) -> Vec<String> {
    let plurals = plural_bases(source);
    let mut keys = Vec::new();
    let mut done = BTreeSet::new();
    for (key, _) in source {
        let base = key.rsplit_once('.').map(|(b, _)| b).filter(|b| plurals.contains(*b));
        match base {
            Some(base) if done.insert(base.to_owned()) => {
                keys.extend(plural::categories(tag).iter().map(|c| format!("{base}.{}", c.name())))
            }
            Some(_) => {}
            None => keys.push(key.clone()),
        }
    }
    keys
}

/// The canonical text of a catalogue.
fn render(entries: &[(String, Option<String>)]) -> String {
    let mut out = String::from("{\n");
    for (i, (key, text)) in entries.iter().enumerate() {
        let value = text.as_ref().map_or_else(
            || "null".to_owned(),
            |t| serde_json::to_string(t).expect("test: a string serializes"),
        );
        let key = serde_json::to_string(key).expect("test: a key serializes");
        let comma = if i + 1 < entries.len() { "," } else { "" };
        let _ = writeln!(out, "  {key}: {value}{comma}");
    }
    out.push_str("}\n");
    out
}

/// `tag`'s catalogue as it must be: the expected keys in order, with the
/// texts it has.
fn canonical(tag: &str, source: &Entries, current: &Entries) -> Vec<(String, Option<String>)> {
    let texts: BTreeMap<&str, &Option<String>> =
        current.iter().map(|(k, t)| (k.as_str(), t)).collect();
    expected_keys(tag, source)
        .into_iter()
        .map(|key| {
            let text = texts.get(key.as_str()).and_then(|t| (*t).clone());
            (key, text)
        })
        .collect()
}

#[test]
fn every_shipped_locale_has_one_canonical_catalogue_with_the_source_keys() {
    let source = source();
    assert!(source.len() > 1_000, "parsed {} source keys", source.len());
    let mut files = catalogues();
    for locale in SHIPPED {
        let current = files.remove(locale.tag).unwrap_or_default();
        let wanted = canonical(locale.tag, &source, &current);
        let text = render(&wanted);
        let path = catalogue_path(locale.tag);
        if syncing() {
            fs::write(&path, &text).expect("test: catalogue writes");
            continue;
        }
        let on_disk = fs::read_to_string(&path).unwrap_or_default();
        assert!(
            on_disk == text,
            "{} is not canonical or its keys differ from en.json; run \
             RAMFLUX_LOCALES_SYNC=1 cargo test -p ramflux-desktop-ui --test locales -- \
             --test-threads=1",
            path.display()
        );
    }
    let strays: Vec<&String> = files.keys().collect();
    assert!(strays.is_empty(), "catalogues of locales that do not ship: {strays:?}");
}

#[test]
fn english_and_simplified_chinese_are_complete_with_no_empty_text() {
    for (tag, entries) in catalogues().into_iter().filter(|(t, _)| t == "en" || t == "zh-CN") {
        for (key, text) in entries {
            let text = text.unwrap_or_else(|| panic!("{key} is not translated in {tag}"));
            let separator = key.starts_with("format.");
            assert!(separator || !text.trim().is_empty(), "{key} ({tag}) has empty text");
        }
    }
    for (tag, entries) in catalogues() {
        for (key, text) in entries {
            // A separator may be a space (a narrow no-break space groups
            // French digits); copy is never blank.
            let blank = text.is_some_and(|t| t.trim().is_empty() && !key.starts_with("format."));
            assert!(!blank, "{key} ({tag}): empty, use null");
        }
    }
}

#[test]
fn english_has_no_cjk_and_no_three_dots() {
    for (key, text) in source() {
        let text = text.unwrap_or_default();
        let endonym = key.starts_with("locale.name.");
        assert!(endonym || !text.chars().any(is_cjk), "English text of {key} contains CJK: {text}");
        assert!(!text.contains("..."), "{key} uses three dots instead of an ellipsis");
    }
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

fn is_key(s: &str) -> bool {
    s.contains('.')
        && s.split('.').all(|part| {
            !part.is_empty()
                && part.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

/// Every string literal in `dirs` shaped like a translation key whose first
/// segment is a top-level group of the catalogue (this also catches keys
/// selected in `match` arms before being passed to `tr`).
fn used_keys(dirs: &[PathBuf], groups: &BTreeSet<String>) -> BTreeSet<String> {
    let mut files = Vec::new();
    for dir in dirs {
        rust_files(dir, &mut files);
    }
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

fn groups(source: &Entries) -> BTreeSet<String> {
    source.iter().filter_map(|(k, _)| k.split('.').next()).map(str::to_owned).collect()
}

#[test]
fn every_key_used_in_code_exists() {
    let source = source();
    let keys: BTreeSet<&str> = source.iter().map(|(k, _)| k.as_str()).collect();
    let used = used_keys(&[crate_dir().join("src")], &groups(&source));
    assert!(used.len() > 100, "found {} keys in code", used.len());
    // A plural message is used by its base key and defined per category.
    let missing: Vec<&String> = used
        .iter()
        .filter(|key| {
            !keys.contains(key.as_str()) && !keys.contains(format!("{key}.other").as_str())
        })
        .collect();
    assert!(missing.is_empty(), "keys used in code but missing in the catalogue: {missing:?}");
}

/// Prefixes the code builds keys under at run time (`format!` with an enum
/// key, or a reason code from the SDK), so their keys are used even when no
/// literal names them.
const DYNAMIC_PREFIXES: [&str; 12] = [
    "agents.unavailable.",
    "backend.backup.",
    "backend.failure.action.",
    "backend.message.",
    "backend.service.",
    // MLS groups: the keys of error codes, join refusals and send states come
    // from the core (`mls_group::error_key`, `JoinRefusal::key`, `QueueReason::key`).
    "group.error.",
    "group.join.refused.",
    "group.send.",
    "sections.tasks.",
    "settings.section.",
    "theme.family.",
    "theme.name.",
];

#[test]
fn every_key_is_used() {
    let source = source();
    let plurals = plural_bases(&source);
    let workspace = crate_dir().join("../..");
    let used = used_keys(
        &[crate_dir().join("src"), workspace.join("crates/ramflux-desktop-sdk/src")],
        &groups(&source),
    );
    let names: BTreeSet<&str> = ramflux_desktop_ui::i18n::locale::SHIPPED
        .iter()
        .chain(ramflux_desktop_ui::i18n::locale::PSEUDO.iter())
        .map(|l| l.name_key)
        .collect();
    let unused: Vec<&String> = source
        .iter()
        .map(|(k, _)| k)
        .filter(|key| {
            let base = key.rsplit_once('.').map(|(b, _)| b).filter(|b| plurals.contains(*b));
            let literal = used.contains(*key) || base.is_some_and(|b| used.contains(b));
            !literal
                && !names.contains(key.as_str())
                && !DYNAMIC_PREFIXES.iter().any(|p| key.starts_with(p))
        })
        .collect();
    assert!(unused.is_empty(), "keys no code uses (delete them): {unused:?}");
}

fn placeholders(text: &str) -> BTreeSet<String> {
    text.split("%{").skip(1).filter_map(|p| p.split_once('}')).map(|(n, _)| n.to_owned()).collect()
}

#[test]
fn placeholders_agree_with_english_in_every_locale() {
    let source = source();
    let plurals = plural_bases(&source);
    let english: BTreeMap<&str, &str> =
        source.iter().map(|(k, t)| (k.as_str(), t.as_deref().unwrap_or_default())).collect();
    let mut checked = 0;
    for (tag, entries) in catalogues() {
        for (key, text) in &entries {
            let Some(text) = text else { continue };
            let plural = key.rsplit_once('.').filter(|(b, _)| plurals.contains(*b));
            let (reference, may_omit_count) = match plural {
                // Every form of a plural message takes the arguments of the
                // English `other`; a form other than `other` may leave out
                // the count ("one message" in a language that spells it).
                Some((base, category)) => {
                    (english[format!("{base}.other").as_str()], category != "other")
                }
                None => (english[key.as_str()], false),
            };
            let mut wanted = placeholders(reference);
            let found = placeholders(text);
            if may_omit_count && !found.contains("count") {
                wanted.remove("count");
            }
            assert_eq!(found, wanted, "{key} ({tag}) must take the arguments of English: {text}");
            checked += 1;
        }
    }
    assert!(checked > 3_000, "checked {checked} texts");
    assert!(plurals.len() >= 20, "found {} plural messages", plurals.len());
}

#[test]
fn the_runtime_resolves_the_source_catalogue() {
    ramflux_desktop_ui::i18n::set_locale(SOURCE);
    for (key, text) in source().into_iter().step_by(17) {
        let Some(text) = text else { continue };
        // The build leaves the card gallery's strings out without its feature.
        if key.starts_with("card_gallery.") && !cfg!(feature = "card-gallery") {
            continue;
        }
        if text.contains('%')
            || key.rsplit_once('.').is_some_and(|(_, c)| Category::parse(c).is_some())
        {
            continue;
        }
        assert_eq!(ramflux_desktop_ui::i18n::tr(&key).as_ref(), text, "{key} is not loaded");
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
    let source = source();
    let keys: BTreeSet<&str> = source.iter().map(|(k, _)| k.as_str()).collect();
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
        assert!(keys.contains(key), "{key} has no translation");
    }
}

#[test]
fn a_reason_with_arguments_is_translated_with_them() {
    ramflux_desktop_ui::i18n::set_locale(SOURCE);
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
    for (tag, entries) in catalogues() {
        for (key, text) in entries {
            let Some(text) = text else { continue };
            assert!(!card_id(&text), "{key} ({tag}) names an internal card id: {text}");
            for term in PROTOCOL_TERMS {
                assert!(
                    !text.to_lowercase().contains(&term.to_lowercase()),
                    "{key} ({tag}) uses the protocol term {term:?}: {text}"
                );
            }
        }
    }
}

#[test]
fn a_plural_message_follows_the_count_and_formats_it() {
    use ramflux_desktop_ui::i18n::{self, tr_args, tr_count};
    i18n::set_locale(SOURCE);
    assert_eq!(tr_count("sections.groups.members", 1).as_ref(), "1 member");
    assert_eq!(tr_count("sections.groups.members", 3).as_ref(), "3 members");
    assert_eq!(tr_count("sections.groups.members", 12_345).as_ref(), "12,345 members");
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
    let keys = ["format.time", "format.date", "format.month_day", "format.date_time"];
    for (tag, entries) in catalogues() {
        for (key, text) in entries.iter().filter(|(k, _)| keys.contains(&k.as_str())) {
            let Some(pattern) = text else { continue };
            let items = chrono::format::StrftimeItems::new(pattern);
            assert!(
                !items.into_iter().any(|item| matches!(item, chrono::format::Item::Error)),
                "{key} ({tag}) is not a valid date pattern: {pattern}"
            );
        }
    }
}

/// The coverage report: per locale, how many entries are translated.
fn coverage_report() -> String {
    let source = source();
    let files = catalogues();
    let mut out = String::from(
        "# Desktop locale coverage\n\n\
         Generated by `RAMFLUX_LOCALES_SYNC=1 cargo test -p ramflux-desktop-ui --test locales\n\
         -- --test-threads=1`; do not edit. An untranslated entry shows the English text. Formatting\n\
         data (`format.*`: number separators, date patterns) counts as translated\n\
         where the catalogue sets it.\n\n\
         | Locale | Direction | Entries | Translated | Untranslated | Coverage |\n\
         | --- | --- | ---: | ---: | ---: | ---: |\n",
    );
    for locale in SHIPPED {
        let entries = canonical(locale.tag, &source, files.get(locale.tag).unwrap_or(&Vec::new()));
        let total = entries.len();
        let translated = entries.iter().filter(|(_, t)| t.is_some()).count();
        let direction = if locale.direction.is_rtl() { "RTL" } else { "LTR" };
        let tenths = translated * 1000 / total.max(1);
        let _ = writeln!(
            out,
            "| `{}` | {direction} | {total} | {translated} | {} | {}.{}% |",
            locale.tag,
            total - translated,
            tenths / 10,
            tenths % 10
        );
    }
    out
}

#[test]
fn the_coverage_report_is_current() {
    let path = crate_dir().join("../../docs/i18n-coverage.md");
    let report = coverage_report();
    if syncing() {
        fs::write(&path, &report).expect("test: report writes");
        return;
    }
    let on_disk = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        on_disk == report,
        "{} is stale; run RAMFLUX_LOCALES_SYNC=1 cargo test -p ramflux-desktop-ui --test \
         locales -- --test-threads=1",
        path.display()
    );
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
        "hard-coded user-visible text (use the catalogues):\n{}",
        found.join("\n")
    );
}
