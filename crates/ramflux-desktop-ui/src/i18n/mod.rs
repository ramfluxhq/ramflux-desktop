// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Interface strings and locale-aware labels.
//!
//! Every user-facing string comes from the files in `locales/` (`zh-CN`
//! default, `en` provided). Code and comments never contain translated text;
//! keys describe stable intent.
//!
//! - [`labels`]: names of contract enums (lifecycle, risk, roles, states).
//! - [`clock`]: local wall-clock times, day headers, relative times and
//!   sizes, in the patterns of the current locale.
//! - [`format`]: numbers, plurals and composed labels.
//!
//! A message whose text depends on a count is a plural message: its key
//! has one child per CLDR plural category (`one`, `other`), and
//! [`tr_args`] picks the child for the `count` argument (or `total`). Count
//! arguments are formatted as numbers of the current locale.

pub mod clock;
pub mod format;
pub mod labels;

use gpui_kit::component::input::InputState;
use gpui_kit::{App, AppContext as _, Context, Entity, Global, SharedString, Subscription, Window};
use ramflux_desktop_core::{EffectFailure, Store};

pub use clock::{bytes, date_time, day_label, offset, relative, same_day, size, time_of_day};
pub use format::{clause, clauses, join, join_all, labeled, list, number, percent, sentences};
pub use labels::*;

/// Locale used when nothing else is selected and the system's language is
/// not known (tests, screenshots).
pub const DEFAULT_LOCALE: &str = "zh-CN";

/// Supported locales and the key of each locale's own name.
pub const LOCALES: [(&str, &str); 2] = [("zh-CN", "locale.chinese"), ("en", "locale.english")];

/// The locale of the shipped ones that serves a language tag (`zh-Hans-CN`,
/// `en_US.UTF-8`): any Chinese tag gets `zh-CN`, everything else English.
pub fn resolve_locale(tag: &str) -> &'static str {
    let language = tag.split(['-', '_', '.']).next().unwrap_or_default();
    if language.eq_ignore_ascii_case("zh") { "zh-CN" } else { "en" }
}

/// The shipped locale serving the operating system's preferred language.
pub fn system_locale() -> &'static str {
    sys_locale::get_locale().as_deref().map_or("en", resolve_locale)
}

/// The text of `key` in the current locale (or its fallback), if any.
fn lookup(key: &str) -> Option<String> {
    crate::_rust_i18n_try_translate(&locale(), key).map(std::borrow::Cow::into_owned)
}

/// Looks up a translated string for the current locale.
pub fn tr(key: &str) -> SharedString {
    SharedString::from(rust_i18n::t!(key).into_owned())
}

/// Arguments that are counts: formatted as numbers of the locale.
const COUNT_ARGS: [&str; 4] = ["count", "total", "done", "bad"];

/// Looks up a translated string and substitutes `%{name}` placeholders.
///
/// When `key` is a plural message, the child for the `count` argument (else
/// `total`) is used. Count arguments that are whole numbers are written as
/// numbers of the current locale.
pub fn tr_args(key: &str, args: &[(&str, &str)]) -> SharedString {
    let operand = ["count", "total"]
        .iter()
        .find_map(|name| args.iter().find(|(n, _)| n == name))
        .and_then(|(_, value)| parse_count(value));
    let plural = operand.and_then(|n| {
        lookup(&format!("{key}.{}", format::category(n)))
            .or_else(|| lookup(&format!("{key}.other")))
    });
    let mut text = plural
        .or_else(|| lookup(key))
        .or_else(|| lookup(&format!("{key}.other")))
        .unwrap_or_else(|| rust_i18n::t!(key).into_owned());
    for (name, value) in args {
        let count = COUNT_ARGS.contains(name).then(|| parse_count(value)).flatten();
        let value = count.map_or_else(|| (*value).to_owned(), format::number);
        text = text.replace(&format!("%{{{name}}}"), &value);
    }
    SharedString::from(text)
}

/// A count argument as a number: plain digits, or digits already grouped
/// with the locale's separator.
fn parse_count(value: &str) -> Option<u64> {
    let group = tr("format.number.group");
    let digits =
        if group.is_empty() { value.to_owned() } else { value.replace(group.as_ref(), "") };
    digits.parse::<u64>().ok()
}

/// A plural message for `n`, with `n` as its `count` argument.
pub fn tr_count(key: &str, n: u64) -> SharedString {
    tr_args(key, &[("count", &n.to_string())])
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

/// What a failed request was about, in words (never the method name): the
/// family of the method (`call.answer` is a call), or a generic word.
pub fn action_name(method: &str) -> SharedString {
    tr(&format!("backend.failure.action.{}", action_family(method)))
}

/// The locale family of `method`'s action word (`other` for an unknown one).
fn action_family(method: &str) -> &'static str {
    match method.split_once('.').map_or(method, |(family, _)| family) {
        "restore" | "backup" => "backup",
        "context" | "conversation" => "conversation",
        "send_message" | "message" => "message",
        "account" => "account",
        "call" => "call",
        "contact" => "contact",
        "device" => "device",
        "grant" => "grant",
        "group" => "group",
        "handoff" => "handoff",
        "node" => "node",
        "object" => "object",
        "surface" => "surface",
        "task" => "task",
        _ => "other",
    }
}

/// One failed effect for the status strip: the action in words, then the
/// reason. When the reason is itself about that kind of object (the key
/// `backend.contact.…` for a contact action), the action word would only
/// repeat it ("Contact: Contact requests can't be handled…"), so the reason
/// stands alone.
pub fn failure(failure: &EffectFailure) -> SharedString {
    let reason = backend_text(&failure.message);
    let family = action_family(&failure.method);
    let names_the_object = failure
        .message
        .strip_prefix("backend.")
        .and_then(|rest| rest.strip_prefix(family))
        .is_some_and(|rest| rest.starts_with('.'));
    if names_the_object {
        return reason;
    }
    tr_args(
        "backend.failure.value",
        &[("action", &action_name(&failure.method)), ("reason", &reason)],
    )
}
