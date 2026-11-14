// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Appearance: theme and language. A local preference, not an
//! intent: nothing here reaches the store or the source. Each choice
//! applies at once and is saved on this device (see [`crate::preferences`]).

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _, StyledExt as _, ThemeMode, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, SharedString, div, rems};
use ramflux_desktop_navigation::theme::{self, ThemeInfo};
use ramflux_desktop_sdk::local::FOLLOW_SYSTEM;

use crate::i18n::{LOCALES, tr, tr_args};

use crate::preferences::{self, LanguageChoice, ThemeChoice};

/// The name of a registered theme: its translation when the locale files
/// name it (`theme.name.<id>`), else the name its token set gives itself.
pub fn theme_name(info: &ThemeInfo) -> SharedString {
    let key = format!("theme.name.{}", info.id);
    let translated = tr(&key);
    if translated.ends_with(key.as_str()) { info.name.clone() } else { translated }
}

/// A theme as the picker lists it: a family of token sets that differ only
/// in mode (light and dark), named after its sets without the mode word.
struct Family {
    /// The sets' shared name ("Ramflux" for "Ramflux Light/Dark").
    key: String,
    /// The light set's id, if the family has one.
    light: Option<SharedString>,
    /// The dark set's id, if the family has one.
    dark: Option<SharedString>,
}

impl Family {
    /// The family's set for `mode`, else its other one.
    fn set_for(&self, mode: ThemeMode) -> Option<SharedString> {
        match mode {
            ThemeMode::Dark => self.dark.clone().or_else(|| self.light.clone()),
            ThemeMode::Light => self.light.clone().or_else(|| self.dark.clone()),
        }
    }

    /// The name shown: its translation (`theme.family.<key>`) when the
    /// locale files have one, else the sets' own name.
    fn label(&self) -> SharedString {
        let key = format!("theme.family.{}", self.key.to_lowercase().replace(' ', "_"));
        let translated = tr(&key);
        if translated.ends_with(key.as_str()) { self.key.clone().into() } else { translated }
    }
}

/// The registered token sets grouped into families, in registration order.
fn families(cx: &App) -> Vec<Family> {
    let mut families: Vec<Family> = Vec::new();
    for info in theme::themes(cx) {
        let key = info.name.trim_end_matches(" Light").trim_end_matches(" Dark").trim().to_owned();
        let index = match families.iter().position(|f| f.key == key) {
            Some(index) => index,
            None => {
                families.push(Family { key, light: None, dark: None });
                families.len() - 1
            }
        };
        if let Some(family) = families.get_mut(index) {
            match info.mode {
                ThemeMode::Light => family.light = Some(info.id.clone()),
                ThemeMode::Dark => family.dark = Some(info.id.clone()),
            }
        }
    }
    families
}

/// The mode the appearance choice asks for: a fixed one, or none when it
/// follows the system.
fn chosen_mode(cx: &App) -> Option<ThemeMode> {
    match preferences::theme_choice(cx) {
        ThemeChoice::System => None,
        ThemeChoice::Named(id) => {
            theme::themes(cx).into_iter().find(|info| info.id == id).map(|info| info.mode)
        }
    }
}

/// The appearance section (design G13 / I12): a one-line note; the Display
/// group with the Mode control (Light / Dark / Follow system), the theme
/// list (every token-file theme) and the interface language; then the
/// preview of the type and controls in the current theme.
pub fn page(_: &App) -> SettingPage {
    SettingPage::new(tr("settings.section.appearance")).group(
        SettingGroup::new()
            .item(SettingItem::render(|_, _, cx: &mut App| body(cx).into_any_element())),
    )
}

fn body(cx: &App) -> impl IntoElement {
    let palette = ramflux_desktop_navigation::palette(cx);
    let rem = |px: f32| rems(px / 15.0);
    let label = |key: &str| {
        div()
            .text_size(rem(palette.size_secondary))
            .font_medium()
            .text_color(palette.text)
            .child(tr(key))
    };
    let helper = |text: SharedString| {
        div().text_size(rem(palette.size_secondary)).text_color(palette.secondary).child(text)
    };
    let heading =
        |key: &str| div().text_size(rem(palette.size_subheading)).font_semibold().child(tr(key));
    let section = |id: &'static str| {
        v_flex().id(id).gap(rem(20.0)).pb(rem(20.0)).border_b_1().border_color(palette.line)
    };
    // Mode.
    let chosen = chosen_mode(cx);
    let painting = if cx.theme().is_dark() { ThemeMode::Dark } else { ThemeMode::Light };
    let families = families(cx);
    let active = theme::active(cx);
    let current_family = active
        .as_ref()
        .and_then(|a| {
            families
                .iter()
                .position(|f| f.light.as_ref() == Some(&a.id) || f.dark.as_ref() == Some(&a.id))
        })
        .unwrap_or(0);
    let family_for_mode =
        families.get(current_family).map(|f| (f.light.clone(), f.dark.clone())).unwrap_or_default();
    let segment = |id: &'static str, key: &'static str, on: bool, choice: Option<ThemeChoice>| {
        Button::new(id)
            .ghost()
            .small()
            .flex_1()
            .h(rem(28.0))
            .rounded(gpui_kit::px(palette.radius_xs))
            .when(on, |b| b.bg(palette.surface_white).font_semibold())
            .selected(on)
            .toggled(on)
            .label(tr(key))
            .accessibility_label(tr_args(
                "settings.appearance.field",
                &[("label", &tr("settings.appearance.mode")), ("value", &tr(key))],
            ))
            .on_click(move |_, _, cx| {
                if let Some(choice) = choice.clone() {
                    preferences::set_theme(choice, cx);
                }
            })
    };
    let mode_help = match chosen {
        None => tr_args(
            "settings.appearance.mode_help_system",
            &[(
                "current",
                &tr(if painting == ThemeMode::Dark {
                    "settings.appearance.mode_dark"
                } else {
                    "settings.appearance.mode_light"
                }),
            )],
        ),
        Some(ThemeMode::Dark) => tr("settings.appearance.mode_help_dark"),
        Some(ThemeMode::Light) => tr("settings.appearance.mode_help_light"),
    };
    let mode = v_flex()
        .gap_2()
        .child(label("settings.appearance.mode"))
        .child(
            h_flex()
                .id("appearance-mode")
                .test_support()
                .role(gpui_kit::Role::RadioGroup)
                .aria_label(tr("settings.appearance.mode"))
                .w(rem(360.0))
                .p(gpui_kit::px(2.))
                .gap(gpui_kit::px(2.))
                .rounded(rem(palette.radius_sm))
                .bg(palette.surface)
                .child(segment(
                    "appearance-mode-light",
                    "settings.appearance.mode_light",
                    chosen == Some(ThemeMode::Light),
                    family_for_mode.0.clone().map(ThemeChoice::Named),
                ))
                .child(segment(
                    "appearance-mode-dark",
                    "settings.appearance.mode_dark",
                    chosen == Some(ThemeMode::Dark),
                    family_for_mode.1.clone().map(ThemeChoice::Named),
                ))
                .child(segment(
                    "appearance-mode-system",
                    "settings.appearance.mode_system",
                    chosen.is_none(),
                    Some(ThemeChoice::System),
                )),
        )
        .child(helper(mode_help));
    // Theme list.
    let mode_now = chosen.unwrap_or(painting);
    let list = v_flex()
        .id("appearance-themes")
        .role(gpui_kit::Role::RadioGroup)
        .aria_label(tr("settings.theme"))
        .rounded(rem(palette.radius_input))
        .border_1()
        .border_color(palette.line_strong)
        .bg(palette.surface_white)
        .children(families.iter().enumerate().map(|(ix, family)| {
            let selected = ix == current_family;
            let target = family.set_for(mode_now);
            let follows = chosen.is_none();
            let name = family.label();
            Button::new(gpui_kit::SharedString::from(format!("appearance-theme-{ix}")))
                .ghost()
                .w_full()
                .h_auto()
                .p(rem(12.0))
                // Board G13: the chosen theme is marked by its radio on the
                // white list, not by a selected fill.
                .toggled(selected)
                .accessibility_label(name.clone())
                .on_click(move |_, _, cx| {
                    // Following the system keeps following it; the default
                    // family is the one the system choice paints.
                    if !follows && let Some(id) = target.clone() {
                        preferences::set_theme(ThemeChoice::Named(id), cx);
                    }
                })
                .child(
                    h_flex()
                        .w_full()
                        .gap_3()
                        .child(
                            div()
                                .size(rem(18.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(rem(palette.radius_full))
                                .border_1()
                                .border_color(palette.primary)
                                .when(selected, |radio| {
                                    radio.child(
                                        div()
                                            .size(rem(8.0))
                                            .rounded(rem(palette.radius_full))
                                            .bg(palette.primary),
                                    )
                                }),
                        )
                        .child(div().text_size(rem(palette.size_body)).font_medium().child(name))
                        .when(ix == 0, |row| {
                            row.child(
                                div()
                                    .px_2()
                                    .py(rem(4.0))
                                    .rounded(rem(palette.radius_chip))
                                    .bg(palette.surface)
                                    .text_size(rem(palette.size_meta))
                                    .child(tr("settings.appearance.default")),
                            )
                        })
                        .child(div().flex_1())
                        .child(h_flex().gap_1().children(
                            [palette.bg, palette.selected, palette.outgoing, palette.text].map(
                                |color| {
                                    div()
                                        .size(rem(16.0))
                                        .rounded(rem(palette.radius_full))
                                        .border_1()
                                        .border_color(palette.line_strong)
                                        .bg(color)
                                },
                            ),
                        )),
                )
        }));
    let theme = v_flex()
        .gap_2()
        .child(label("settings.theme"))
        .child(list)
        .child(helper(tr("settings.appearance.theme_help")));
    // Interface language: the whole field opens the list.
    let current_language = match preferences::language_choice(cx) {
        LanguageChoice::System => tr("locale.system"),
        LanguageChoice::Locale(code) => LOCALES
            .iter()
            .find(|(c, _)| *c == code)
            .map_or_else(|| tr("locale.system"), |(_, key)| tr(key)),
    };
    let language = v_flex()
        .gap_2()
        .child(label("settings.language"))
        .child(
            Button::new("appearance-language")
                .outline()
                .w_full()
                .h(rem(40.0))
                .px_3()
                .justify_start()
                .rounded(gpui_kit::px(palette.radius_input))
                .accessibility_label(tr_args(
                    "settings.appearance.field",
                    &[("label", &tr("settings.language")), ("value", &current_language)],
                ))
                .child(
                    h_flex()
                        .w_full()
                        .text_size(rem(palette.size_body))
                        .child(div().flex_1().child(current_language))
                        .child(
                            gpui_kit::component::Icon::new(
                                gpui_kit::component::IconName::ChevronDown,
                            )
                            .small(),
                        ),
                )
                .dropdown_menu(|menu, _, _| {
                    language_options().into_iter().fold(menu, |menu, (code, name)| {
                        menu.item(gpui_kit::component::menu::PopupMenuItem::new(name).on_click(
                            move |_, _, cx| {
                                preferences::set_language(LanguageChoice::parse(&code), cx)
                            },
                        ))
                    })
                }),
        )
        .child(helper(tr("settings.appearance.language_help")));
    let unsaved = preferences::unsaved(cx).map(|reason| super::note_text(reason, cx));
    v_flex()
        .id("appearance")
        .w_full()
        .gap(rem(24.0))
        .child(
            div()
                .text_size(rem(palette.size_body))
                .text_color(palette.secondary)
                .child(tr("settings.appearance.local_only")),
        )
        .child(
            section("appearance-display")
                .child(heading("settings.appearance.display"))
                .child(mode)
                .child(theme)
                .child(language)
                .children(unsaved),
        )
        .child(section("appearance-preview").gap_3().children(preview(cx)))
}

/// The language options: follow the system, then every shipped locale.
fn language_options() -> Vec<(SharedString, SharedString)> {
    let mut options = vec![(SharedString::new_static(FOLLOW_SYSTEM), tr("locale.system"))];
    options.extend(LOCALES.iter().map(|(code, key)| (SharedString::new_static(code), tr(key))));
    options
}

/// The preview of G13: a heading, body text, a status tag and the primary
/// and secondary buttons, all in the current theme.
fn preview(cx: &App) -> Vec<gpui_kit::AnyElement> {
    let palette = ramflux_desktop_navigation::palette(cx);
    let rem = |px: f32| rems(px / 15.0);
    vec![
        div()
            .text_size(rem(palette.size_subheading))
            .font_semibold()
            .child(tr("settings.appearance.preview"))
            .into_any_element(),
        div()
            .text_size(rem(palette.size_page))
            .font_semibold()
            .child(tr("settings.appearance.preview_heading"))
            .into_any_element(),
        div()
            .text_size(rem(palette.size_body))
            .child(tr("settings.appearance.preview_body"))
            .into_any_element(),
        h_flex()
            .child(
                div()
                    .px_2()
                    .py(rem(4.0))
                    .rounded(rem(palette.radius_chip))
                    .bg(palette.surface)
                    .text_size(rem(palette.size_meta))
                    .child(tr("settings.appearance.preview_tag")),
            )
            .into_any_element(),
        h_flex()
            .gap_2()
            .child(
                crate::style::primary_button("appearance-preview-primary", cx)
                    .label(tr("settings.appearance.preview_primary")),
            )
            .child(
                crate::style::secondary_button("appearance-preview-secondary", cx)
                    .label(tr("settings.appearance.preview_secondary")),
            )
            .into_any_element(),
    ]
}
