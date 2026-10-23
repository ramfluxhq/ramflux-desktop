// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Local wall-clock formatting: times of day, day headers, dates and sizes.
//!
//! Contract timestamps are unix milliseconds; they are shown in the
//! machine's local time zone. Digits are locale-neutral.

use chrono::{DateTime, Local, NaiveDate, TimeZone as _};
use gpui_kit::SharedString;
use ramflux_desktop_core::Timestamp;

use super::{tr, tr_args};

fn local(at: Timestamp) -> Option<DateTime<Local>> {
    Local.timestamp_millis_opt(at.millis()).single()
}

/// Local calendar day of a timestamp.
pub fn local_day(at: Timestamp) -> Option<NaiveDate> {
    local(at).map(|t| t.date_naive())
}

/// Local time of day as `HH:MM`.
pub fn time_of_day(at: Timestamp) -> String {
    local(at).map_or_else(|| "--:--".to_owned(), |t| t.format("%H:%M").to_string())
}

/// Local date and time as `YYYY-MM-DD HH:MM`.
pub fn date_time(at: Timestamp) -> String {
    local(at).map_or_else(String::new, |t| t.format("%Y-%m-%d %H:%M").to_string())
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
        _ => tr_args("clock.date", &[("date", &day.format("%Y-%m-%d").to_string())]),
    }
}

/// A scenario offset as `m:ss.t`.
pub fn offset(ms: u64) -> String {
    let tenths = (ms / 100) % 10;
    let seconds = (ms / 1_000) % 60;
    let minutes = ms / 60_000;
    format!("{minutes}:{seconds:02}.{tenths}")
}

/// A byte count with a binary unit, one decimal above KiB.
pub fn bytes(size: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if size < 1024 {
        return format!("{size} B");
    }
    // Lossy by design: sizes are only displayed.
    #[allow(clippy::cast_precision_loss)]
    let mut value = size as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_local_wall_clock_times() {
        let at = Timestamp(1_790_812_800_000 + 3_600_000 + 5 * 60_000);
        let expected = Local.timestamp_millis_opt(at.millis()).single().expect("valid");
        assert_eq!(time_of_day(at), expected.format("%H:%M").to_string());
        assert_ne!(time_of_day(Timestamp(at.millis() + 7 * 60_000)), time_of_day(at));
    }

    #[test]
    fn day_labels_are_relative_to_now() {
        let now = Timestamp(1_790_812_800_000);
        assert_eq!(day_label(now, now), tr("clock.today"));
        assert_eq!(day_label(Timestamp(now.millis() - 86_400_000), now), tr("clock.yesterday"));
        let older = day_label(Timestamp(now.millis() - 5 * 86_400_000), now);
        assert!(older.contains("2026-09-2"), "{older}");
    }

    #[test]
    fn offsets_and_sizes_are_stable() {
        assert_eq!(offset(61_250), "1:01.2");
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(482_133), "470.8 KiB");
        assert_eq!(bytes(73_400_320), "70.0 MiB");
    }
}
