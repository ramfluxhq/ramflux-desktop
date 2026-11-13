// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Appearance: theme and language. A local preference, not an
//! intent: nothing here reaches the store or the source. Each choice
//! applies at once and is saved on this device (see [`crate::preferences`]).

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, StyledExt as _, ThemeMode, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, SharedString, div, rems};
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

/// The appearance section (design G13): a one-line note, the Display group
/// with the dark-theme row and the full theme picker (follow the system or
/// any registered token set), the interface language, and a preview of the
/// type and controls in the current theme.
pub fn page(cx: &App) -> SettingPage {
    SettingPage::new(tr("settings.section.appearance"))
        .group(
            SettingGroup::new()
                .item(SettingItem::render(|_, _, cx: &mut App| {
                    note("settings.appearance.local_only", cx)
                }))
                .item(SettingItem::render(|_, _, cx: &mut App| {
                    section_title("settings.appearance.display", cx)
                }))
                .item(SettingItem::render(|_, _, cx: &mut App| dark_row(cx).into_any_element()))
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
        .group(
            SettingGroup::new()
                .item(SettingItem::render(|_, _, cx: &mut App| preview(cx).into_any_element())),
        )
}

/// A group heading in the subheading size.
fn section_title(key: &str, cx: &App) -> gpui_kit::AnyElement {
    let palette = ramflux_desktop_navigation::palette(cx);
    div()
        .text_size(rems(palette.size_subheading / 15.0))
        .font_semibold()
        .child(tr(key))
        .into_any_element()
}

/// "Dark theme": whether the dark set paints now, and one button that turns
/// it on or off (a named light or dark set; Follow system stays in the
/// picker below).
fn dark_row(cx: &App) -> impl IntoElement {
    let palette = ramflux_desktop_navigation::palette(cx);
    let choice = preferences::theme_choice(cx);
    let dark = cx.theme().is_dark();
    let state = match (&choice, dark) {
        (ThemeChoice::System, true) => tr("settings.appearance.dark_system_on"),
        (ThemeChoice::System, false) => tr("settings.appearance.dark_system_off"),
        (_, true) => tr("settings.appearance.state_on"),
        (_, false) => tr("settings.appearance.state_off"),
    };
    let target = if dark { ThemeMode::Light } else { ThemeMode::Dark };
    h_flex()
        .id("appearance-dark")
        .w_full()
        .gap_3()
        .pb_3()
        .border_b_1()
        .border_color(palette.line)
        .child(
            v_flex()
                .flex_1()
                .child(
                    div()
                        .text_size(rems(palette.size_body / 15.0))
                        .font_medium()
                        .child(tr("settings.appearance.dark")),
                )
                .child(
                    div()
                        .text_size(rems(palette.size_secondary / 15.0))
                        .text_color(palette.secondary)
                        .child(state),
                ),
        )
        .child(
            Button::new("appearance-dark-toggle")
                .outline()
                .small()
                .label(tr(if dark {
                    "settings.appearance.turn_off"
                } else {
                    "settings.appearance.turn_on"
                }))
                .on_click(move |_, _, cx| {
                    let id = theme::themes(cx)
                        .into_iter()
                        .find(|info| info.mode == target)
                        .map(|info| info.id);
                    if let Some(id) = id {
                        preferences::set_theme(ThemeChoice::Named(id), cx);
                    }
                }),
        )
}

/// The preview of G13: a heading, body text, a status tag and the primary
/// and secondary buttons, all in the current theme.
fn preview(cx: &App) -> impl IntoElement {
    let palette = ramflux_desktop_navigation::palette(cx);
    v_flex()
        .id("appearance-preview")
        .gap_3()
        .child(section_title("settings.appearance.preview", cx))
        .child(
            div()
                .text_size(rems(palette.size_page / 15.0))
                .font_bold()
                .child(tr("settings.appearance.preview_heading")),
        )
        .child(
            div()
                .text_size(rems(palette.size_body / 15.0))
                .child(tr("settings.appearance.preview_body")),
        )
        .child(
            h_flex().child(
                div()
                    .px_2()
                    .py(gpui_kit::px(2.))
                    .rounded(rems(palette.radius_chip / 15.0))
                    .bg(palette.surface)
                    .text_size(rems(palette.size_meta / 15.0))
                    .child(tr("settings.appearance.preview_tag")),
            ),
        )
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("appearance-preview-primary")
                        .primary()
                        .small()
                        .label(tr("settings.appearance.preview_primary")),
                )
                .child(
                    Button::new("appearance-preview-secondary")
                        .outline()
                        .small()
                        .label(tr("settings.appearance.preview_secondary")),
                ),
        )
}
