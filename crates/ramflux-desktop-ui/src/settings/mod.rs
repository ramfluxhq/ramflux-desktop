// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The settings page (P2): a full-window overlay with the section list.
//!
//! Appearance (theme, language) works in this build; the other sections are
//! placeholders until their cards land. MCP and A2I are readonly by design.

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::setting::{
    SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _, Theme, ThemeMode, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, SharedString, Window, div};

use crate::actions::{CloseSettings, SETTINGS_CONTEXT};
use crate::i18n::{self, LOCALES, tr};

/// The settings overlay.
#[derive(IntoElement)]
pub struct SettingsPage;

fn placeholder_page(title_key: &str, readonly: bool) -> SettingPage {
    let note = tr(if readonly { "settings.readonly_later" } else { "settings.later" });
    SettingPage::new(tr(title_key)).group(SettingGroup::new().item(SettingItem::render(
        move |_, _, cx: &mut App| {
            div().text_sm().text_color(cx.theme().muted_foreground).child(note.clone())
        },
    )))
}

fn appearance_page() -> SettingPage {
    let languages: Vec<(SharedString, SharedString)> =
        LOCALES.iter().map(|(code, key)| (SharedString::from(*code), tr(key))).collect();
    SettingPage::new(tr("settings.appearance")).default_open(true).group(
        SettingGroup::new()
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

impl RenderOnce for SettingsPage {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        v_flex()
            .id("settings")
            .key_context(SETTINGS_CONTEXT)
            .absolute()
            .inset_0()
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .px_4()
                    .py_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().flex_1().font_semibold().child(tr("settings.title")))
                    .child(
                        Button::new("settings-close")
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .label(tr("common.close"))
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(CloseSettings), cx)
                            }),
                    ),
            )
            .child(div().flex_1().min_h_0().child(Settings::new("settings-pages").pages([
                placeholder_page("settings.account", false),
                placeholder_page("settings.devices", false),
                placeholder_page("settings.node", false),
                placeholder_page("settings.integrations", true),
                placeholder_page("settings.diagnostics", false),
                appearance_page(),
            ])))
    }
}
