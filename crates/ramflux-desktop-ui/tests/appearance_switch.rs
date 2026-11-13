// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI integration test of Settings › Appearance at runtime: picking a theme
//! or a language in the open window changes the colour tokens and the
//! visible text at once (no restart), "follow system" follows the
//! operating system's appearance and language, and both choices are saved
//! on the device and read back.
//!
//! Its own test binary: the locale and the theme are process-wide, so
//! switching them must not race the other UI tests.

mod common;

use std::path::PathBuf;

use common::*;
use gpui_kit::component::{ActiveTheme as _, ThemeMode};
use gpui_kit::test::TestEventExt as _;
use gpui_kit::test::TestQueryExt as _;
use gpui_kit::{Hsla, MouseButton, TestAppContext, WindowAppearance};
use ramflux_desktop_navigation::theme;
use ramflux_desktop_sdk::local::{AppearanceSettings, AppearanceStore};
use ramflux_desktop_ui::i18n::{self, tr};
use ramflux_desktop_ui::preferences::{self, LanguageChoice, ThemeChoice};
use ramflux_desktop_ui::settings::SettingsSection;

type Handle = gpui_kit::WindowHandle<gpui_kit::base::Root>;

/// Clicks the element whose accessible label is `label`.
fn click_label(cx: &mut TestAppContext, handle: Handle, label: &str) {
    let label = label.to_owned();
    in_window(cx, handle, move |window, cx| {
        let center = window.find_by_label(&label).bounds().center();
        window.pointer_down(center, MouseButton::Left, cx);
        window.pointer_up(center, MouseButton::Left, cx);
    });
}

/// Whether an element labelled `label` is in the window.
fn shows(cx: &mut TestAppContext, handle: Handle, label: &str) -> bool {
    let label = label.to_owned();
    in_window(cx, handle, move |window, _| window.try_find_by_label(&label).is_some())
}

/// The colour tokens a view paints with: Kit's background and foreground,
/// the RF rail and the generated palette's outgoing bubble.
fn tokens(cx: &mut TestAppContext) -> (Hsla, Hsla, Hsla, Hsla) {
    cx.update(|cx| {
        (
            cx.theme().background,
            cx.theme().foreground,
            theme::colors(cx).rail,
            ramflux_desktop_navigation::palette(cx).outgoing,
        )
    })
}

/// The visible label of the theme dropdown showing `value`.
fn theme_field(value: &str) -> String {
    format!("{}: {value}", tr("settings.theme"))
}

/// The visible label of the language dropdown showing `value`.
fn language_field(value: &str) -> String {
    format!("{}: {value}", tr("settings.language"))
}

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rf-appearance-ui-{name}-{}", std::process::id()))
}

#[gpui_kit::test]
fn theme_and_language_switch_in_the_open_window_and_follow_the_system(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 1_000);
    cx.update(|cx| {
        shell.update(cx, |shell, cx| shell.open_settings_at(SettingsSection::Appearance, cx))
    });
    cx.run_until_parked();
    assert_eq!(i18n::locale(), i18n::DEFAULT_LOCALE, "tests start in the default locale");
    let light = tokens(cx);
    assert!(!cx.update(|cx| cx.theme().is_dark()), "tests start light");

    // Theme: Light -> Dark from the dropdown, at once.
    click_label(cx, handle, &theme_field(&tr("theme.name.light")));
    click_label(cx, handle, &tr("theme.name.dark"));
    let dark = tokens(cx);
    assert!(cx.update(|cx| cx.theme().is_dark()), "the dark set is active without a restart");
    assert_ne!(dark.0, light.0, "Kit background follows the token set");
    assert_ne!(dark.1, light.1, "Kit foreground follows the token set");
    assert_ne!(dark.2, light.2, "RF rail follows the token set");
    assert_ne!(dark.3, light.3, "the generated palette follows the token set");
    assert_eq!(cx.update(|cx| preferences::theme_choice(cx)), ThemeChoice::parse("dark"));
    assert!(shows(cx, handle, &theme_field(&tr("theme.name.dark"))), "the picker shows it");

    // Language: Chinese -> English, the visible text changes at once.
    let chinese_title = tr("settings.section.appearance");
    assert!(shows(cx, handle, &language_field(&tr("locale.chinese"))));
    click_label(cx, handle, &language_field(&tr("locale.chinese")));
    click_label(cx, handle, &tr("locale.english"));
    assert_eq!(i18n::locale(), "en", "English is active without a restart");
    assert_ne!(tr("settings.section.appearance"), chinese_title);
    assert!(shows(cx, handle, &language_field("English")), "the field reads in English now");
    assert!(shows(cx, handle, &theme_field("Dark")), "every label is English now");

    // Follow system: the language is the system's; the theme follows the
    // operating system's appearance both ways.
    click_label(cx, handle, &language_field("English"));
    click_label(cx, handle, &tr("locale.system"));
    assert_eq!(cx.update(|cx| preferences::language_choice(cx)), LanguageChoice::System);
    assert_eq!(i18n::locale(), i18n::system_locale(), "the system's language is used");
    click_label(cx, handle, &theme_field(&tr("theme.name.dark")));
    click_label(cx, handle, &tr("settings.theme_system"));
    assert_eq!(cx.update(|cx| preferences::theme_choice(cx)), ThemeChoice::System);
    cx.update(|cx| preferences::appearance_changed(WindowAppearance::Dark, cx));
    cx.run_until_parked();
    assert_eq!(tokens(cx), dark, "the system turned dark: the dark token set");
    cx.update(|cx| preferences::appearance_changed(WindowAppearance::Light, cx));
    cx.run_until_parked();
    assert_eq!(tokens(cx), light, "the system turned light: the light token set");

    // A fixed theme ignores the system.
    cx.update(|cx| preferences::set_theme(ThemeChoice::parse("light"), cx));
    cx.update(|cx| preferences::appearance_changed(WindowAppearance::Dark, cx));
    cx.run_until_parked();
    assert_eq!(tokens(cx), light, "a chosen theme stays when the system changes");
    cx.update(|cx| preferences::set_language(LanguageChoice::Locale(i18n::DEFAULT_LOCALE), cx));
}

#[gpui_kit::test]
fn choices_are_saved_on_the_device_and_read_back(cx: &mut TestAppContext) {
    let dir = scratch("saved");
    let path = dir.join("desktop-settings.json");
    cx.update(|cx| {
        gpui_kit::init(cx);
        ramflux_desktop_ui::init(cx);
        preferences::install(AppearanceStore::at(path.clone()), cx);
    });
    assert_eq!(
        cx.update(|cx| preferences::theme_choice(cx)),
        ThemeChoice::System,
        "nothing saved yet: the theme follows the system"
    );
    assert_eq!(cx.update(|cx| preferences::language_choice(cx)), LanguageChoice::System);
    cx.update(|cx| {
        preferences::set_theme(ThemeChoice::parse("dark"), cx);
        preferences::set_language(LanguageChoice::Locale("en"), cx);
    });
    assert!(cx.update(|cx| cx.theme().is_dark()));
    assert_eq!(
        AppearanceStore::at(path.clone()).load(),
        AppearanceSettings { theme: Some("dark".to_owned()), language: Some("en".to_owned()) },
        "both choices are in this device's settings file"
    );
    // A restart reads them back.
    cx.update(|cx| {
        theme::select_mode(ThemeMode::Light, cx);
        preferences::install(AppearanceStore::at(path.clone()), cx);
    });
    assert!(cx.update(|cx| cx.theme().is_dark()), "the saved theme applies on start");
    assert_eq!(i18n::locale(), "en", "the saved language applies on start");
    assert!(cx.update(|cx| preferences::unsaved(cx)).is_none(), "saving worked");
    // A theme this build does not ship falls back to the system's appearance.
    AppearanceStore::at(path.clone())
        .save(&AppearanceSettings { theme: Some("retired".to_owned()), language: None })
        .expect("test: settings written");
    cx.update(|cx| preferences::install(AppearanceStore::at(path.clone()), cx));
    let expected = cx.update(|cx| ThemeMode::from(cx.window_appearance()));
    assert_eq!(cx.update(|cx| cx.theme().mode), expected, "an unknown theme follows the system");
    cx.update(preferences::install_fixed);
    cx.update(|cx| preferences::set_language(LanguageChoice::Locale(i18n::DEFAULT_LOCALE), cx));
    let _ = std::fs::remove_dir_all(dir);
}
