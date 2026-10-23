// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Appearance: theme and language. A local preference, not an
//! intent: nothing here reaches the store or the source.

use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode};
use gpui_kit::{App, SharedString};

use super::note;
use crate::i18n::{self, LOCALES, tr};

/// The appearance section.
pub fn page() -> SettingPage {
    let languages: Vec<(SharedString, SharedString)> =
        LOCALES.iter().map(|(code, key)| (SharedString::from(*code), tr(key))).collect();
    SettingPage::new(tr("settings.section.appearance")).group(
        SettingGroup::new()
            .item(SettingItem::render(|_, _, cx: &mut App| note("settings.local_only", cx)))
            .item(SettingItem::new(
                tr("settings.dark_theme"),
                SettingField::switch(
                    |cx: &App| cx.theme().is_dark(),
                    |dark: bool, cx: &mut App| {
                        Theme::change(
                            if dark { ThemeMode::Dark } else { ThemeMode::Light },
                            None,
                            cx,
                        );
                        cx.refresh_windows();
                    },
                ),
            ))
            .item(SettingItem::new(
                tr("settings.language"),
                SettingField::dropdown(
                    languages,
                    |_: &App| SharedString::from(i18n::locale()),
                    |locale: SharedString, cx: &mut App| i18n::select_locale(&locale, cx),
                ),
            )),
    )
}
