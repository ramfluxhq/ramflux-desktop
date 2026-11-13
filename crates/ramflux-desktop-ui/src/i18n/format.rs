// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Locale-aware formatting of numbers, plurals and composed labels.
//!
//! Every separator, unit and pattern comes from the locale files (the
//! `format.*` keys), so a new locale changes them without code changes:
//!
//! - [`number`] groups digits (`format.number.group`) and [`decimal`] uses
//!   the locale's decimal mark (`format.number.decimal`).
//! - [`plural_category`] picks the CLDR plural category of a count; a key
//!   with `one`/`other` children is a plural message (see
//!   [`super::tr_args`]).
//! - [`join`], [`sentences`], [`labeled`] and [`percent`] compose labels
//!   from translated parts the way each language writes them.

use gpui_kit::SharedString;

use super::{locale, tr, tr_args};

/// The CLDR plural category of `n` in `locale`: `one` or `other` (the only
/// categories the shipped locales use; Chinese has no plural forms).
pub fn plural_category(locale: &str, n: u64) -> &'static str {
    let language = locale.split(['-', '_']).next().unwrap_or(locale);
    match language {
        "zh" | "ja" | "ko" => "other",
        _ if n == 1 => "one",
        _ => "other",
    }
}

/// The plural category of `n` in the current locale.
pub fn category(n: u64) -> &'static str {
    plural_category(&locale(), n)
}

/// `digits` (ASCII digits only) with the locale's group separator every
/// three digits from the right.
fn grouped(digits: &str, separator: &str) -> String {
    let len = digits.chars().count();
    let mut out = String::with_capacity(digits.len() + separator.len() * (len / 3));
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push_str(separator);
        }
        out.push(c);
    }
    out
}

/// A count in the current locale (`12,345`).
pub fn number(n: u64) -> String {
    grouped(&n.to_string(), &tr("format.number.group"))
}

/// A non-negative decimal with `places` decimals in the current locale
/// (`470.8`).
pub fn decimal(value: f64, places: usize) -> String {
    let text = format!("{value:.places$}");
    let (whole, fraction) = text.split_once('.').unwrap_or((text.as_str(), ""));
    let whole = if whole.bytes().all(|b| b.is_ascii_digit()) {
        number_text(whole)
    } else {
        whole.to_owned()
    };
    if fraction.is_empty() {
        whole
    } else {
        format!("{whole}{}{fraction}", tr("format.number.decimal"))
    }
}

fn number_text(digits: &str) -> String {
    grouped(digits, &tr("format.number.group"))
}

/// A share of a whole (`progress` in 0..=1) as a whole percentage in the
/// current locale (`42%`).
pub fn percent(progress: f64) -> SharedString {
    let value = (progress.clamp(0.0, 1.0) * 100.0).round();
    tr_args("format.percent", &[("value", &decimal(value, 0))])
}

/// Two parts of one label side by side (`a · b`).
pub fn join(a: &str, b: &str) -> SharedString {
    tr_args("format.join", &[("a", a), ("b", b)])
}

/// A title followed by its description, as two sentences (`a. b`): the
/// accessible name of a state block.
pub fn sentences(a: &str, b: &str) -> SharedString {
    tr_args("format.sentences", &[("a", a), ("b", b)])
}

/// A name and what is said about it (`a: b`).
pub fn labeled(a: &str, b: &str) -> SharedString {
    tr_args("format.labeled", &[("a", a), ("b", b)])
}

/// Two clauses read as one phrase (`a, b`): an accessible name made of a
/// title and its detail.
pub fn clause(a: &str, b: &str) -> SharedString {
    tr_args("format.clause", &[("a", a), ("b", b)])
}

/// Folds `items` pairwise with `pair`; empty for no items.
fn fold<S: AsRef<str>>(items: &[S], pair: fn(&str, &str) -> SharedString) -> SharedString {
    let mut parts = items.iter().map(AsRef::as_ref);
    let Some(first) = parts.next() else { return SharedString::default() };
    parts.fold(SharedString::from(first.to_owned()), |text, next| pair(&text, next))
}

/// Parts of one label side by side (`a · b · c`).
pub fn join_all<S: AsRef<str>>(items: &[S]) -> SharedString {
    fold(items, join)
}

/// Clauses read as one phrase (`a, b, c`): an accessible name.
pub fn clauses<S: AsRef<str>>(items: &[S]) -> SharedString {
    fold(items, clause)
}

/// An enumeration of names, joined with the locale's list separator
/// (`format.list`).
pub fn list<S: AsRef<str>>(items: &[S]) -> SharedString {
    let separator = tr("format.list");
    let parts: Vec<&str> = items.iter().map(AsRef::as_ref).collect();
    SharedString::from(parts.join(separator.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_has_one_and_other_and_chinese_only_other() {
        assert_eq!(plural_category("en", 1), "one");
        assert_eq!(plural_category("en", 0), "other");
        assert_eq!(plural_category("en", 2), "other");
        assert_eq!(plural_category("zh-CN", 1), "other", "Chinese has no singular form");
    }

    #[test]
    fn digits_are_grouped_from_the_right() {
        assert_eq!(grouped("1", ","), "1");
        assert_eq!(grouped("1234", ","), "1,234");
        assert_eq!(grouped("123456", ","), "123,456");
        assert_eq!(grouped("1234567", "\u{a0}"), "1\u{a0}234\u{a0}567");
    }
}
