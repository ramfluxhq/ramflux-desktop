// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Locale-aware formatting: the one module for dates, times, relative
//! times, numbers, percentages, byte sizes, plurals and composed labels.
//!
//! Every separator, unit and pattern comes from the catalogues (the
//! `format.*` keys), so a new locale changes them without code changes:
//!
//! - [`number`] groups digits (`format.number.group`, in groups of
//!   `format.number.group_sizes`: `3`, or `3,2` for the Indian system) and
//!   [`decimal`] uses the locale's decimal mark (`format.number.decimal`).
//! - [`time_of_day`], [`date`], [`date_time`], [`day_label`] and [`relative`]
//!   write a local wall-clock time in the locale's strftime patterns
//!   (`format.time`, `format.date`, `format.month_day`, `format.date_time`)
//!   and relative phrases (`clock.*`).
//! - [`bytes`] writes a byte count with binary units (`format.size.*`).
//! - [`join`], [`sentences`], [`labeled`], [`clause`], [`list`] and
//!   [`percent`] compose labels from translated parts the way each language
//!   writes them.
//!
//! Digits are Western Arabic digits in every locale. Contract timestamps are
//! unix milliseconds shown in the machine's local time zone.

use chrono::{DateTime, Local, NaiveDate, TimeZone as _};
use gpui_kit::SharedString;
use ramflux_desktop_core::Timestamp;

use super::{locale, plural, tr, tr_args, tr_count};

/// The CLDR plural category name of `n` in `locale`.
pub fn plural_category(locale: &str, n: u64) -> &'static str {
    plural::category(locale, n).name()
}

/// The plural category name of `n` in the current locale.
pub fn category(n: u64) -> &'static str {
    plural_category(locale::current().tag, n)
}

/// `digits` (ASCII digits only) with `separator` between groups: the first
/// group from the right has `sizes[0]` digits, every later one `sizes[1]`
/// (or `sizes[0]` again).
fn grouped(digits: &str, separator: &str, sizes: (usize, usize)) -> String {
    let (first, rest) = (sizes.0.max(1), sizes.1.max(1));
    let len = digits.chars().count();
    let mut out = String::with_capacity(digits.len() + separator.len() * (len / 2));
    for (i, c) in digits.chars().enumerate() {
        let from_right = len - i;
        let boundary = from_right > first && (from_right - first).is_multiple_of(rest);
        if i > 0 && (from_right == first || boundary) {
            out.push_str(separator);
        }
        out.push(c);
    }
    out
}

/// The locale's group sizes (`3`, or `3,2`).
fn group_sizes() -> (usize, usize) {
    let sizes = tr("format.number.group_sizes");
    let mut parts = sizes.split(',').filter_map(|s| s.trim().parse::<usize>().ok());
    let first = parts.next().unwrap_or(3);
    (first, parts.next().unwrap_or(first))
}

fn number_text(digits: &str) -> String {
    grouped(digits, &tr("format.number.group"), group_sizes())
}

/// A count in the current locale (`12,345`).
pub fn number(n: u64) -> String {
    number_text(&n.to_string())
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

fn local(at: Timestamp) -> Option<DateTime<Local>> {
    Local.timestamp_millis_opt(at.millis()).single()
}

/// Local calendar day of a timestamp.
pub fn local_day(at: Timestamp) -> Option<NaiveDate> {
    local(at).map(|t| t.date_naive())
}

/// Whether `a` and `b` fall on the same local calendar day.
pub fn same_day(a: Timestamp, b: Timestamp) -> bool {
    matches!((local_day(a), local_day(b)), (Some(a), Some(b)) if a == b)
}

/// `pattern` (a locale key holding a strftime pattern) applied to `at`.
fn formatted<Tz: chrono::TimeZone>(at: &DateTime<Tz>, key: &str) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let pattern = tr(key);
    let items: Vec<chrono::format::Item<'_>> =
        chrono::format::StrftimeItems::new(&pattern).collect();
    if items.iter().any(|item| matches!(item, chrono::format::Item::Error)) {
        return at.format("%Y-%m-%d %H:%M").to_string();
    }
    at.format_with_items(items.into_iter()).to_string()
}

/// Local time of day in the locale's pattern (`14:05`, `2:05 PM`).
pub fn time_of_day(at: Timestamp) -> String {
    local(at)
        .map_or_else(|| tr("format.time_unknown").to_string(), |t| formatted(&t, "format.time"))
}

/// Local date and time in the locale's pattern.
pub fn date_time(at: Timestamp) -> String {
    local(at).map_or_else(String::new, |t| formatted(&t, "format.date_time"))
}

/// Local date in the locale's pattern.
pub fn date(at: Timestamp) -> String {
    local(at).map_or_else(String::new, |t| formatted(&t, "format.date"))
}

/// Header for the local day of `at`, relative to the local day of `now`:
/// today, yesterday, or the date.
pub fn day_label(at: Timestamp, now: Timestamp) -> SharedString {
    let (Some(day), Some(today)) = (local_day(at), local_day(now)) else {
        return SharedString::default();
    };
    // Design `RF / Date separator`: the relative day with its month and
    // day ("Today · Nov 10").
    let short = local(at).map_or_else(String::new, |t| formatted(&t, "format.month_day"));
    match (today - day).num_days() {
        0 => tr_args("clock.relative_day", &[("day", &tr("clock.today")), ("date", &short)]),
        1 => tr_args("clock.relative_day", &[("day", &tr("clock.yesterday")), ("date", &short)]),
        _ => tr_args("clock.date", &[("date", &date(at))]),
    }
}

/// How long ago `at` was, seen at `now`, as a phrase that reads inside a
/// sentence ("Last seen …"): just now, minutes or hours ago, yesterday, or
/// on the date. A time in the future reads as just now.
pub fn relative(at: Timestamp, now: Timestamp) -> SharedString {
    const MINUTE: i64 = 60_000;
    const HOUR: i64 = 60 * MINUTE;
    let elapsed = now.millis().saturating_sub(at.millis());
    let whole = |unit: i64| u64::try_from(elapsed / unit).unwrap_or_default();
    let days = match (local_day(at), local_day(now)) {
        (Some(day), Some(today)) => (today - day).num_days(),
        _ => 0,
    };
    if elapsed < MINUTE {
        tr("clock.relative.just_now")
    } else if elapsed < HOUR {
        tr_count("clock.relative.minutes_ago", whole(MINUTE))
    } else if days <= 0 {
        tr_count("clock.relative.hours_ago", whole(HOUR))
    } else if days == 1 {
        tr("clock.relative.yesterday")
    } else {
        tr_args("clock.relative.on_date", &[("date", &date(at))])
    }
}

/// A scenario offset as `m:ss.t`.
pub fn offset(ms: u64) -> String {
    let tenths = (ms / 100) % 10;
    let seconds = (ms / 1_000) % 60;
    let minutes = ms / 60_000;
    format!("{minutes}:{seconds:02}.{tenths}")
}

/// A size that may be unknown: the byte count, or a note that the
/// backend did not report it.
pub fn size(size: Option<u64>) -> String {
    size.map_or_else(|| tr("moment.object.size_unknown").to_string(), bytes)
}

/// A byte count with a binary unit, one decimal above a kibibyte, in the
/// locale's number format and unit names.
pub fn bytes(size: u64) -> String {
    const UNITS: [&str; 4] =
        ["format.size.kib", "format.size.mib", "format.size.gib", "format.size.tib"];
    if size < 1024 {
        return tr_args("format.size.bytes", &[("value", &number(size))]).to_string();
    }
    let mut divisor: u64 = 1024;
    let mut unit = 0;
    while unit + 1 < UNITS.len() && size / divisor >= 1024 {
        divisor = divisor.saturating_mul(1024);
        unit += 1;
    }
    // Tenths of the unit, rounded half up, in integers.
    let tenths = (u128::from(size) * 10 + u128::from(divisor) / 2) / u128::from(divisor);
    let whole = u64::try_from(tenths / 10).unwrap_or(u64::MAX);
    let value = format!("{}{}{}", number(whole), tr("format.number.decimal"), tenths % 10);
    let key = UNITS.get(unit).copied().unwrap_or("format.size.tib");
    tr_args(key, &[("value", &value)]).to_string()
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
        assert_eq!(plural_category("ar", 2), "two");
    }

    #[test]
    fn digits_are_grouped_from_the_right() {
        assert_eq!(grouped("1", ",", (3, 3)), "1");
        assert_eq!(grouped("123", ",", (3, 3)), "123");
        assert_eq!(grouped("1234", ",", (3, 3)), "1,234");
        assert_eq!(grouped("123456", ",", (3, 3)), "123,456");
        assert_eq!(grouped("1234567", "\u{a0}", (3, 3)), "1\u{a0}234\u{a0}567");
    }

    #[test]
    fn the_indian_system_groups_by_two_after_the_first_three() {
        assert_eq!(grouped("1234", ",", (3, 2)), "1,234");
        assert_eq!(grouped("12345", ",", (3, 2)), "12,345");
        assert_eq!(grouped("1234567", ",", (3, 2)), "12,34,567");
        assert_eq!(grouped("123456789", ",", (3, 2)), "12,34,56,789");
    }

    #[test]
    fn times_are_local_wall_clock_times() {
        let at = Timestamp(1_790_812_800_000 + 3_600_000 + 5 * 60_000);
        let expected = Local.timestamp_millis_opt(at.millis()).single().expect("valid");
        assert_eq!(time_of_day(at), expected.format(&tr("format.time")).to_string());
        assert_ne!(time_of_day(Timestamp(at.millis() + 7 * 60_000)), time_of_day(at));
    }

    #[test]
    fn day_labels_are_relative_to_now() {
        let now = Timestamp(1_790_812_800_000);
        assert!(day_label(now, now).starts_with(tr("clock.today").as_ref()));
        assert!(
            day_label(Timestamp(now.millis() - 86_400_000), now)
                .starts_with(tr("clock.yesterday").as_ref())
        );
        let older = day_label(Timestamp(now.millis() - 5 * 86_400_000), now);
        assert!(older.contains("2026"), "an older day shows its date: {older}");
    }

    #[test]
    fn offsets_and_sizes_are_stable() {
        assert_eq!(offset(61_250), "1:01.2");
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(482_133), "470.8 KiB");
        assert_eq!(bytes(73_400_320), "70.0 MiB");
    }

    #[test]
    fn relative_times_step_from_minutes_to_hours_to_days() {
        let now = Timestamp(1_790_812_800_000 + 12 * 3_600_000);
        let at = |ms: i64| Timestamp(now.millis() - ms);
        let ago = |ms: i64| relative(at(ms), now);
        assert_eq!(ago(10_000), tr("clock.relative.just_now"), "under a minute");
        assert_eq!(ago(-5_000), tr("clock.relative.just_now"), "a clock ahead is not the future");
        assert_eq!(ago(5 * 60_000), tr_count("clock.relative.minutes_ago", 5));
        assert_eq!(ago(60_000), tr_count("clock.relative.minutes_ago", 1));
        if same_day(at(2 * 3_600_000), now) {
            assert_eq!(ago(2 * 3_600_000), tr_count("clock.relative.hours_ago", 2));
        }
        let older = ago(3 * 86_400_000);
        assert_eq!(
            older,
            tr_args("clock.relative.on_date", &[("date", &date(at(3 * 86_400_000)))])
        );
    }
}
