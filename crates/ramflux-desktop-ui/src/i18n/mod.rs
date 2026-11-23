// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Interface strings and locale-aware labels.
//!
//! Every user-facing string comes from the catalogues in `locales/`, one
//! flat JSON file per locale (`en.json` is the source). Code and comments
//! never contain translated text; keys describe stable intent. The build
//! compiles the catalogues into static tables and fails when a key written
//! in the code is missing from the English catalogue (`build.rs`).
//!
//! - [`locale`]: the shipped locales, system-tag resolution, the fallback
//!   chain (locale, language, English) and the current locale.
//! - [`plural`]: CLDR plural categories for every shipped language.
//! - [`bidi`]: isolation of untrusted text, left-to-right identifiers and
//!   the direction of a message.
//! - [`pseudo`]: the long and right-to-left pseudo-locales.
//! - [`format`]: dates, times, relative times, numbers, sizes and list
//!   joins in the patterns of the current locale.
//! - [`labels`]: names of contract enums (lifecycle, risk, roles, states).
//!
//! A message whose text depends on a count is a plural message: its key
//! has one child per CLDR plural category its language uses (`one`, `few`,
//! `other`, …), and [`tr_args`] picks the child for the `count` argument (or
//! `total`). A locale serves a plural message only when it has every
//! category its language needs; otherwise the next locale of the chain
//! serves it whole. Count arguments are formatted as numbers of the locale.
//! Every other argument is isolated when the interface reads right to left
//! or the argument holds right-to-left text or directional controls.

pub mod bidi;
mod catalog;
pub mod format;
pub mod labels;
pub mod locale;
pub mod plural;
pub mod pseudo;

use std::borrow::Cow;

use gpui_kit::component::input::InputState;
use gpui_kit::{App, AppContext as _, Context, Entity, Global, SharedString, Subscription, Window};
use ramflux_desktop_core::{EffectFailure, Store};
use ramflux_desktop_navigation::direction;

pub use format::{
    bytes, clause, clauses, date_time, day_label, join, join_all, labeled, list, number, offset,
    percent, relative, same_day, sentences, size, time_of_day,
};
pub use labels::*;
pub use locale::Locale;

/// Locale used when nothing else is selected and the system's language is
/// not known (tests, screenshots).
pub const DEFAULT_LOCALE: &str = "zh-CN";

/// Every locale this build lets the user pick, in picker order (the
/// pseudo-locales only in developer builds).
pub fn locales() -> impl Iterator<Item = &'static Locale> {
    locale::selectable()
}

/// The shipped locale serving a language tag (`zh-Hans-CN`, `en_US.UTF-8`).
pub fn resolve_locale(tag: &str) -> &'static str {
    locale::resolve(tag).tag
}

/// The shipped locale serving the operating system's preferred language.
pub fn system_locale() -> &'static str {
    sys_locale::get_locale().as_deref().map_or(locale::SOURCE, resolve_locale)
}

/// The text of `key` in catalogue `tag`: for a count, the plural form when
/// `tag` has every category of its language (else the plain key).
fn in_catalogue(tag: &str, key: &str, count: Option<u64>) -> Option<&'static str> {
    let form =
        |category: plural::Category| catalog::get(tag, &format!("{key}.{}", category.name()));
    let plural = count.and_then(|n| {
        let complete = plural::categories(tag).iter().all(|c| form(*c).is_some());
        complete.then(|| form(plural::category(tag, n))).flatten()
    });
    match count {
        Some(_) => plural.or_else(|| catalog::get(tag, key)),
        None => catalog::get(tag, key).or_else(|| form(plural::Category::Other)),
    }
}

/// The text of `key` in the current locale or the first locale of its
/// fallback chain that has it, as the current locale shows it.
fn resolve(key: &str, count: Option<u64>) -> Option<Cow<'static, str>> {
    let current = locale::current();
    let text = locale::chain(current).into_iter().find_map(|tag| in_catalogue(tag, key, count))?;
    Some(match current.pseudo {
        Some(pseudo) => Cow::Owned(pseudo::transform(pseudo, key, text)),
        None => Cow::Borrowed(text),
    })
}

/// Looks up a translated string for the current locale. A key no catalogue
/// has (a bug the catalogue checks catch) shows as itself.
pub fn tr(key: &str) -> SharedString {
    match resolve(key, None) {
        Some(Cow::Borrowed(text)) => SharedString::new_static(text),
        Some(Cow::Owned(text)) => SharedString::from(text),
        None => SharedString::from(key.to_owned()),
    }
}

/// Arguments that are counts: formatted as numbers of the locale.
const COUNT_ARGS: [&str; 4] = ["count", "total", "done", "bad"];

/// Looks up a translated string and substitutes `%{name}` placeholders.
///
/// When `key` is a plural message, the child for the `count` argument (else
/// `total`) is used. Count arguments that are whole numbers are written as
/// numbers of the current locale; other arguments are isolated where their
/// direction could reorder the message (see [`bidi::argument`]).
pub fn tr_args(key: &str, args: &[(&str, &str)]) -> SharedString {
    let operand = ["count", "total"]
        .iter()
        .find_map(|name| args.iter().find(|(n, _)| n == name))
        .and_then(|(_, value)| parse_count(value));
    let mut text = resolve(key, operand).map_or_else(|| key.to_owned(), Cow::into_owned);
    let interface = direction::layout_direction();
    for (name, value) in args {
        let placeholder = format!("%{{{name}}}");
        if !text.contains(&placeholder) {
            continue;
        }
        let count = COUNT_ARGS.contains(name).then(|| parse_count(value)).flatten();
        let value = count.map_or_else(|| (*value).to_owned(), format::number);
        text = text.replace(&placeholder, &bidi::argument(&value, interface));
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

/// The current locale's tag.
pub fn locale() -> String {
    locale::current().tag.to_owned()
}

/// The current locale.
pub fn current_locale() -> &'static Locale {
    locale::current()
}

/// Makes `tag` the current locale without telling any view: the layout
/// direction and GPUI Kit's own strings follow. For startup and tests;
/// a running interface uses [`select_locale`].
pub fn set_locale(tag: &str) {
    let locale = locale::set_current(tag);
    direction::set_layout_direction(locale.direction);
    ramflux_desktop_navigation::scale::set_cjk_interface(crate::assets::cjk_interface(locale.tag));
    gpui_kit::component::set_locale(locale.kit);
}

/// Marks a locale switch: views observe it to translate again what they
/// keep between frames (input placeholders, table headers, cached reasons).
struct LocaleSwitch;

impl Global for LocaleSwitch {}

/// Selects a locale, tells the views keeping translated state, and redraws
/// every window. The switch applies at once; no restart is needed.
pub fn select_locale(locale: &str, cx: &mut App) {
    set_locale(locale);
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

/// What a catalogue holds, for the coverage report and the checks.
pub mod coverage {
    /// Every key of the source catalogue that has text, sorted.
    pub fn source_keys() -> Vec<&'static str> {
        super::catalog::entries(super::locale::SOURCE).iter().map(|(key, _)| *key).collect()
    }

    /// The translated keys of catalogue `tag`, sorted.
    pub fn translated(tag: &str) -> Vec<&'static str> {
        super::catalog::entries(tag).iter().map(|(key, _)| *key).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_catalogue_serves_a_plural_message_only_with_every_category() {
        let key = "sections.groups.members";
        assert_eq!(in_catalogue("en", key, Some(1)), Some("%{count} member"));
        assert_eq!(in_catalogue("en", key, Some(0)), Some("%{count} members"));
        assert_eq!(in_catalogue("en", key, None), Some("%{count} members"), "no count: `other`");
        let chinese = in_catalogue("zh-CN", key, Some(1));
        assert!(chinese.is_some() && chinese == in_catalogue("zh-CN", key, Some(7)));
        // Not translated yet: the next locale of the chain serves it.
        assert_eq!(in_catalogue("ru", key, Some(3)), None);
        assert_eq!(in_catalogue("ar", key, Some(2)), None);
        assert_eq!(in_catalogue("ar", "common.cancel", None), None);
        assert_eq!(in_catalogue("en", "no.such.key", None), None);
    }

    #[test]
    fn locale_data_is_set_where_it_differs_from_english() {
        assert_eq!(in_catalogue("de", "format.number.group", None), Some("."));
        assert_eq!(in_catalogue("hi", "format.number.group_sizes", None), Some("3,2"));
        assert_eq!(in_catalogue("tr", "format.percent", None), Some("%%{value}"));
    }
}
