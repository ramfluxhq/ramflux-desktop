// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Interface strings and locale-aware labels.
//!
//! Every user-facing string comes from the files in `locales/` (`zh-CN`
//! default, `en` provided). Code and comments never contain translated text;
//! keys describe stable intent.
//!
//! - [`labels`]: names of contract enums (lifecycle, risk, roles, states).
//! - [`clock`]: local wall-clock times, day headers and sizes.

pub mod clock;
pub mod labels;

use gpui_kit::{App, SharedString};

pub use clock::{bytes, date_time, day_label, offset, time_of_day};
pub use labels::*;

/// Locale used when nothing else is selected.
pub const DEFAULT_LOCALE: &str = "zh-CN";

/// Supported locales and the key of each locale's own name.
pub const LOCALES: [(&str, &str); 2] = [("zh-CN", "locale.chinese"), ("en", "locale.english")];

/// Looks up a translated string for the current locale.
pub fn tr(key: &str) -> SharedString {
    SharedString::from(rust_i18n::t!(key).into_owned())
}

/// Looks up a translated string and substitutes `%{name}` placeholders.
pub fn tr_args(key: &str, args: &[(&str, &str)]) -> SharedString {
    let mut text = rust_i18n::t!(key).into_owned();
    for (name, value) in args {
        text = text.replace(&format!("%{{{name}}}"), value);
    }
    SharedString::from(text)
}

/// The current locale.
pub fn locale() -> String {
    rust_i18n::locale().to_string()
}

/// Selects a locale and redraws every window.
pub fn select_locale(locale: &str, cx: &mut App) {
    rust_i18n::set_locale(locale);
    cx.refresh_windows();
}
