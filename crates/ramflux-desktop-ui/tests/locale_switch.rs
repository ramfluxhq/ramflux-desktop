// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI integration test of a language switch from Settings › Appearance: the
//! text the window keeps from the moment it was built (input placeholders)
//! follows the selected locale at once, both ways.
//!
//! Its own test binary: the locale is process-wide, so switching it must not
//! race the other UI tests.

mod common;

use common::*;
use gpui_kit::component::input::InputState;
use gpui_kit::test::TestEventExt as _;
use gpui_kit::test::TestQueryExt as _;
use gpui_kit::{Entity, MouseButton, SharedString, TestAppContext};
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::i18n::{self, tr};
use ramflux_desktop_ui::route::Page;
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

/// Picks `locale` (the code) in the language dropdown of the open
/// Appearance section.
fn pick_language(cx: &mut TestAppContext, handle: Handle, locale: &str) {
    let current = if i18n::locale() == "en" { "locale.english" } else { "locale.chinese" };
    let target = if locale == "en" { "locale.english" } else { "locale.chinese" };
    click_label(cx, handle, &format!("{}: {}", tr("settings.language"), tr(current)));
    click_label(cx, handle, &tr(target));
    assert_eq!(i18n::locale(), locale, "the dropdown selected {locale}");
}

/// The placeholders of window-level inputs: the composer, the entry
/// passphrase and the settings passphrases.
fn placeholders(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Vec<SharedString> {
    cx.update(|cx| {
        let shell = shell.read(cx);
        let mut inputs: Vec<Entity<InputState>> =
            vec![shell.workspace().read(cx).composer().read(cx).input().clone()];
        inputs.push(shell.entry().read(cx).passphrase().clone());
        let settings = shell.settings().read(cx);
        inputs.extend(settings.rotate_inputs().into_iter().cloned());
        inputs.push(settings.backup_input().clone());
        inputs.iter().map(|input| input.read(cx).presentation().placeholder().clone()).collect()
    })
}

/// What [`placeholders`] reads, translated in the active locale.
fn expected() -> Vec<SharedString> {
    [
        "composer.placeholder",
        "entry.account.passphrase",
        "settings.account.current",
        "settings.account.next",
        "settings.account.backup_passphrase",
    ]
    .into_iter()
    .map(tr)
    .collect()
}

#[gpui_kit::test]
fn window_strings_follow_a_language_switch_both_ways(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 1_000);
    assert_eq!(page(cx, &shell), Page::Workspace);
    assert_eq!(i18n::locale(), i18n::DEFAULT_LOCALE);
    let chinese = expected();
    assert_eq!(placeholders(cx, &shell), chinese, "built in the default locale");

    cx.update(|cx| {
        shell.update(cx, |shell, cx| shell.open_settings_at(SettingsSection::Appearance, cx))
    });
    cx.run_until_parked();
    pick_language(cx, handle, "en");
    let english = expected();
    assert_eq!(english[0].as_ref(), "Write a message");
    assert_ne!(english, chinese);
    assert_eq!(placeholders(cx, &shell), english, "switched to English without a restart");

    pick_language(cx, handle, "zh-CN");
    assert_eq!(placeholders(cx, &shell), chinese, "switched back to Chinese");
}
