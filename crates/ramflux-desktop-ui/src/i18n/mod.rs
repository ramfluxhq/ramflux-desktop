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

use gpui_kit::component::input::InputState;
use gpui_kit::{App, AppContext as _, Context, Entity, Global, SharedString, Subscription, Window};
use ramflux_desktop_core::{EffectFailure, Store};

pub use clock::{bytes, date_time, day_label, offset, size, time_of_day};
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

/// Marks a locale switch: views observe it to translate again what they
/// keep between frames (input placeholders, table headers, cached reasons).
struct LocaleSwitch;

impl Global for LocaleSwitch {}

/// Selects a locale, tells the views keeping translated state, and redraws
/// every window.
pub fn select_locale(locale: &str, cx: &mut App) {
    rust_i18n::set_locale(locale);
    cx.set_global(LocaleSwitch);
    cx.refresh_windows();
}

/// Calls `f` on the view after every locale switch.
pub fn observe_locale<T: 'static>(
    window: &Window,
    cx: &mut Context<T>,
    f: impl FnMut(&mut T, &mut Window, &mut Context<T>) + 'static,
) -> Subscription {
    cx.observe_global_in::<LocaleSwitch>(window, f)
}

/// Calls `f` on the view after every locale switch, for views that need no
/// window to translate again.
pub fn on_locale_switch<T: 'static>(
    cx: &mut Context<T>,
    f: impl FnMut(&mut T, &mut Context<T>) + 'static,
) -> Subscription {
    cx.observe_global::<LocaleSwitch>(f)
}

/// A text input whose placeholder is the translation of `key`, translated
/// again after every locale switch for as long as the input lives.
/// `configure` sets everything else (masking, a starting value).
pub fn localized_input(
    key: &'static str,
    window: &mut Window,
    cx: &mut App,
    configure: impl FnOnce(InputState) -> InputState,
) -> Entity<InputState> {
    cx.new(|cx| {
        observe_locale(window, cx, move |input: &mut InputState, window, cx| {
            input.set_placeholder(tr(key), window, cx);
        })
        .detach();
        configure(InputState::new(window, cx).placeholder(tr(key)))
    })
}

/// Why `method` is unavailable on the attached backend, translated, or
/// `None` when it is available. Backend reasons are locale keys.
pub fn unsupported(store: &Store, method: &str) -> Option<SharedString> {
    store.unsupported_reason(method).map(backend_text)
}

/// A backend reason or message: translated when it is a locale key
/// (`backend.*`), else shown as the daemon wrote it. A key may be followed
/// by one space and a JSON object of string arguments, substituted into the
/// translation (`backend.failure.partial {"done":"1","total":"2",…}`).
pub fn backend_text(text: &str) -> SharedString {
    if !text.starts_with("backend.") {
        return SharedString::from(text.to_owned());
    }
    let Some((key, args)) = text.split_once(' ') else { return tr(text) };
    let Ok(serde_json::Value::Object(args)) = serde_json::from_str::<serde_json::Value>(args)
    else {
        return SharedString::from(text.to_owned());
    };
    let args: Vec<(&str, String)> = args
        .iter()
        .map(|(name, value)| {
            let value = value.as_str().map_or_else(|| value.to_string(), str::to_owned);
            (name.as_str(), backend_text(&value).to_string())
        })
        .collect();
    let borrowed: Vec<(&str, &str)> = args.iter().map(|(n, v)| (*n, v.as_str())).collect();
    tr_args(key, &borrowed)
}

/// One failed effect for the status strip.
pub fn failure(failure: &EffectFailure) -> SharedString {
    tr_args(
        "backend.failure.value",
        &[("action", &failure.method), ("reason", &backend_text(&failure.message))],
    )
}
