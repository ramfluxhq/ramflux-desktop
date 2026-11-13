// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Appearance: theme and language. A local preference, not an
//! intent: nothing here reaches the store or the source. Each choice
//! applies at once and is saved on this device (see [`crate::preferences`]).

use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem, SettingPage};
use gpui_kit::{App, IntoElement as _, SharedString};
use ramflux_desktop_navigation::theme::{self, ThemeInfo};
use ramflux_desktop_sdk::local::FOLLOW_SYSTEM;

use super::note;
use crate::i18n::{LOCALES, tr};
use crate::preferences::{self, LanguageChoice, ThemeChoice};

/// The name of a registered theme: its translation when the locale files
/// name it (`theme.name.<id>`), else the name its token set gives itself.
pub fn theme_name(info: &ThemeInfo) -> SharedString {
    let key = format!("theme.name.{}", info.id);
    let translated = tr(&key);
    if translated.ends_with(key.as_str()) { info.name.clone() } else { translated }
}

/// The theme options: follow the system, then every registered theme.
fn theme_options(cx: &App) -> Vec<(SharedString, SharedString)> {
    let mut options = vec![(SharedString::new_static(FOLLOW_SYSTEM), tr("settings.theme_system"))];
    options.extend(theme::themes(cx).iter().map(|info| (info.id.clone(), theme_name(info))));
    options
}

/// The language options: follow the system, then every shipped locale.
fn language_options() -> Vec<(SharedString, SharedString)> {
    let mut options = vec![(SharedString::new_static(FOLLOW_SYSTEM), tr("locale.system"))];
    options.extend(LOCALES.iter().map(|(code, key)| (SharedString::new_static(code), tr(key))));
    options
}

/// The appearance section.
pub fn page(cx: &App) -> SettingPage {
    SettingPage::new(tr("settings.section.appearance")).group(
        SettingGroup::new()
            .item(SettingItem::render(|_, _, cx: &mut App| note("settings.local_only", cx)))
            .item(SettingItem::new(
                tr("settings.theme"),
                SettingField::dropdown(
                    theme_options(cx),
                    |cx: &App| preferences::theme_choice(cx).id(),
                    |id: SharedString, cx: &mut App| {
                        preferences::set_theme(ThemeChoice::parse(&id), cx);
                    },
                ),
            ))
            .item(SettingItem::new(
                tr("settings.language"),
                SettingField::dropdown(
                    language_options(),
                    |cx: &App| preferences::language_choice(cx).id(),
                    |code: SharedString, cx: &mut App| {
                        preferences::set_language(LanguageChoice::parse(&code), cx);
                    },
                ),
            ))
            .item(SettingItem::render(|_, _, cx: &mut App| {
                preferences::unsaved(cx)
                    .map(|reason| super::note_text(reason, cx).into_any_element())
                    .unwrap_or_else(|| gpui_kit::Empty.into_any_element())
            })),
    )
}
