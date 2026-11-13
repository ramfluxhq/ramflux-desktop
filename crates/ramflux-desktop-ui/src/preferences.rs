// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Per-device appearance preferences: the interface theme and language.
//!
//! Each is either a fixed choice or "follow the system". A choice applies
//! at once in every window (no restart) and is saved in the desktop
//! settings file of this device; it is never synced and holds nothing
//! secret. While the theme follows the system, [`appearance_changed`] keeps
//! it in step with the operating system's light/dark appearance.

use gpui_kit::component::ThemeMode;
use gpui_kit::{App, Global, SharedString, WindowAppearance};
use ramflux_desktop_navigation::theme;
use ramflux_desktop_sdk::local::{AppearanceSettings, AppearanceStore, FOLLOW_SYSTEM, app_log};

use crate::i18n;

/// The theme choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeChoice {
    /// Light or dark as the operating system appears.
    System,
    /// A registered theme, by id.
    Named(SharedString),
}

impl ThemeChoice {
    /// The value kept in the settings file and shown by the picker.
    pub fn id(&self) -> SharedString {
        match self {
            Self::System => SharedString::new_static(FOLLOW_SYSTEM),
            Self::Named(id) => id.clone(),
        }
    }

    /// Reads a stored or picked value.
    pub fn parse(value: &str) -> Self {
        if value == FOLLOW_SYSTEM {
            Self::System
        } else {
            Self::Named(SharedString::from(value.to_owned()))
        }
    }
}

/// The language choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanguageChoice {
    /// The operating system's preferred language.
    System,
    /// A shipped locale.
    Locale(&'static str),
}

impl LanguageChoice {
    /// The value kept in the settings file and shown by the picker.
    pub fn id(&self) -> SharedString {
        match self {
            Self::System => SharedString::new_static(FOLLOW_SYSTEM),
            Self::Locale(code) => SharedString::new_static(code),
        }
    }

    /// Reads a stored or picked value; an unknown locale follows the
    /// system.
    pub fn parse(value: &str) -> Self {
        i18n::LOCALES
            .iter()
            .find(|(code, _)| *code == value)
            .map_or(Self::System, |(code, _)| Self::Locale(code))
    }

    /// The locale this choice selects now.
    pub fn locale(&self) -> &'static str {
        match self {
            Self::System => i18n::system_locale(),
            Self::Locale(code) => code,
        }
    }
}

/// The preferences in effect and where they are saved.
struct Preferences {
    store: AppearanceStore,
    theme: ThemeChoice,
    language: LanguageChoice,
    /// Why the last change could not be saved, if it could not.
    unsaved: Option<String>,
}

impl Global for Preferences {}

/// Installs the preferences saved in `store` (anything unset follows the
/// system) and applies them. Call after the theme registry is installed.
pub fn install(store: AppearanceStore, cx: &mut App) {
    let saved = store.load();
    let theme = saved.theme.as_deref().map_or(ThemeChoice::System, ThemeChoice::parse);
    let language = saved.language.as_deref().map_or(LanguageChoice::System, LanguageChoice::parse);
    install_with(store, theme, language, cx);
}

/// Installs fixed preferences that are saved nowhere (tests, screenshots):
/// the current theme mode and the default locale.
pub fn install_fixed(cx: &mut App) {
    let theme = theme::active(cx).map_or(ThemeChoice::System, |info| ThemeChoice::Named(info.id));
    install_with(
        AppearanceStore::in_memory(),
        theme,
        LanguageChoice::Locale(i18n::DEFAULT_LOCALE),
        cx,
    );
}

fn install_with(
    store: AppearanceStore,
    theme: ThemeChoice,
    language: LanguageChoice,
    cx: &mut App,
) {
    cx.set_global(Preferences { store, theme, language, unsaved: None });
    apply_theme(cx);
    rust_i18n::set_locale(language_choice(cx).locale());
}

/// The theme choice in effect.
pub fn theme_choice(cx: &App) -> ThemeChoice {
    cx.try_global::<Preferences>().map_or(ThemeChoice::System, |p| p.theme.clone())
}

/// The language choice in effect.
pub fn language_choice(cx: &App) -> LanguageChoice {
    cx.try_global::<Preferences>().map_or(LanguageChoice::System, |p| p.language.clone())
}

/// Why the last change could not be saved on this device, if it could not.
pub fn unsaved(cx: &App) -> Option<SharedString> {
    cx.try_global::<Preferences>()
        .and_then(|p| p.unsaved.clone())
        .map(|reason| i18n::tr_args("settings.appearance_not_saved", &[("reason", &reason)]))
}

/// Picks a theme: applies it at once in every window and saves it.
pub fn set_theme(choice: ThemeChoice, cx: &mut App) {
    if cx.has_global::<Preferences>() {
        cx.global_mut::<Preferences>().theme = choice;
    }
    apply_theme(cx);
    save(cx);
}

/// Picks a language: applies it at once in every window and saves it.
pub fn set_language(choice: LanguageChoice, cx: &mut App) {
    let locale = choice.locale();
    if cx.has_global::<Preferences>() {
        cx.global_mut::<Preferences>().language = choice;
    }
    save(cx);
    i18n::select_locale(locale, cx);
}

/// The operating system's appearance changed (or a window first learned
/// it): a theme that follows the system follows it.
pub fn appearance_changed(appearance: WindowAppearance, cx: &mut App) {
    if theme_choice(cx) == ThemeChoice::System {
        theme::select_mode(ThemeMode::from(appearance), cx);
        cx.refresh_windows();
    }
}

fn apply_theme(cx: &mut App) {
    match theme_choice(cx) {
        ThemeChoice::System => {
            let appearance = cx.window_appearance();
            theme::select_mode(ThemeMode::from(appearance), cx);
        }
        ThemeChoice::Named(id) => {
            if !theme::select(&id, cx) {
                // A theme saved by a build that shipped it: follow the system.
                let appearance = cx.window_appearance();
                theme::select_mode(ThemeMode::from(appearance), cx);
            }
        }
    }
    cx.refresh_windows();
}

fn save(cx: &mut App) {
    let Some(preferences) = cx.try_global::<Preferences>() else { return };
    let settings = AppearanceSettings {
        theme: Some(preferences.theme.id().to_string()),
        language: Some(preferences.language.id().to_string()),
    };
    let result = preferences.store.save(&settings).map_err(|error| error.to_string());
    if let Err(error) = &result {
        app_log::warn(&format!("appearance not saved: {error}"));
    }
    cx.global_mut::<Preferences>().unsaved = result.err();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_read_back_what_they_write() {
        assert_eq!(ThemeChoice::parse(FOLLOW_SYSTEM), ThemeChoice::System);
        let dark = ThemeChoice::parse("dark");
        assert_eq!(dark, ThemeChoice::Named(SharedString::from("dark")));
        assert_eq!(ThemeChoice::parse(&dark.id()), dark, "a named theme round-trips");
        assert_eq!(LanguageChoice::parse("en"), LanguageChoice::Locale("en"));
        assert_eq!(LanguageChoice::parse("zh-CN").id().as_ref(), "zh-CN");
        assert_eq!(LanguageChoice::parse(FOLLOW_SYSTEM), LanguageChoice::System);
        assert_eq!(
            LanguageChoice::parse("tlh"),
            LanguageChoice::System,
            "a locale this build does not ship follows the system"
        );
    }

    #[test]
    fn a_fixed_language_is_its_locale() {
        assert_eq!(LanguageChoice::Locale("en").locale(), "en");
        let system = LanguageChoice::System.locale();
        assert!(i18n::LOCALES.iter().any(|(code, _)| *code == system), "always a shipped locale");
    }
}
