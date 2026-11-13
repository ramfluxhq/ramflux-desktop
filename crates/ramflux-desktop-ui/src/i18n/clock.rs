// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Local wall-clock formatting: times of day, day headers, dates, relative
//! times and sizes.
//!
//! Contract timestamps are unix milliseconds; they are shown in the
//! machine's local time zone. Patterns (`format.time`, `format.date`,
//! `format.date_time`), relative phrases and size units come from the
//! locale files, so each locale writes them its own way.

use chrono::{DateTime, Local, NaiveDate, TimeZone as _};
use gpui_kit::SharedString;
use ramflux_desktop_core::Timestamp;

use super::{tr, tr_args, tr_count};

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
    match (today - day).num_days() {
        0 => tr("clock.today"),
        1 => tr("clock.yesterday"),
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
        return tr_args("format.size.bytes", &[("value", &super::number(size))]).to_string();
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
    let value = format!("{}{}{}", super::number(whole), tr("format.number.decimal"), tenths % 10);
    let key = UNITS.get(unit).copied().unwrap_or("format.size.tib");
    tr_args(key, &[("value", &value)]).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(day_label(now, now), tr("clock.today"));
        assert_eq!(day_label(Timestamp(now.millis() - 86_400_000), now), tr("clock.yesterday"));
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
