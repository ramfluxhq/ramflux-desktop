// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Representative screens in the pseudo-locales (developer builds): the
//! message list with a conversation open, a section page, the command
//! palette and every settings section render in the long pseudo-locale
//! (`en-XA`) and in the right-to-left one (`ar-XB`). In right to left the
//! navigation rail and the conversation list move to the right edge; in the
//! long pseudo-locale no named element shows a multi-word English catalogue
//! text unchanged (text that bypasses the catalogue).
//!
//! Its own test binary: the locale is process-wide.

mod common;

use std::collections::BTreeSet;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use ramflux_desktop_ui::i18n;
use ramflux_desktop_ui::settings::SettingsSection;
use ramflux_desktop_ui::workspace::WorkspaceRoute;

type Handle = gpui_kit::WindowHandle<gpui_kit::base::Root>;

/// Where an element's center sits, as a share of the window's width.
fn center_share(cx: &mut TestAppContext, handle: Handle, id: &'static str) -> f32 {
    in_window(cx, handle, move |window, _| {
        let bounds = window.find(id).bounds();
        let width = f32::from(window.viewport_size().width);
        f32::from(bounds.center().x) / width
    })
}

/// Multi-word English texts of the source catalogue (without arguments).
fn english_copy() -> BTreeSet<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("locales/en.json");
    let text = std::fs::read_to_string(path).expect("test: en.json reads");
    let value: serde_json::Value = serde_json::from_str(&text).expect("test: en.json parses");
    value
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| !key.starts_with("format.") && !key.starts_with("locale.name."))
        .filter_map(|(_, v)| v.as_str())
        .filter(|t| !t.contains('%') && t.split_whitespace().count() >= 2)
        .map(str::to_owned)
        .collect()
}

/// Named texts of the window that are English catalogue text unchanged.
fn untransformed(cx: &mut TestAppContext, handle: Handle, copy: &BTreeSet<String>) -> Vec<String> {
    in_window(cx, handle, |window, _| {
        window
            .elements()
            .into_iter()
            .flat_map(|e| [e.label().map(str::to_owned), e.value().map(str::to_owned)])
            .flatten()
            .filter(|text| copy.contains(text))
            .collect()
    })
}

fn select(cx: &mut TestAppContext, locale: &str) {
    cx.update(|cx| i18n::select_locale(locale, cx));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn representative_screens_mirror_and_survive_long_text(cx: &mut TestAppContext) {
    if !i18n::locale::pseudo_enabled() {
        return;
    }
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    show_context(cx, handle, &shell, "c-team");

    select(cx, "en");
    let rail = center_share(cx, handle, "navigation-rail");
    assert!(rail < 0.5, "left to right: the rail is on the left ({rail})");

    select(cx, "ar-XB");
    assert!(ramflux_desktop_navigation::direction::is_rtl());
    let rail = center_share(cx, handle, "navigation-rail");
    assert!(rail > 0.5, "right to left: the rail is on the right ({rail})");

    let copy = english_copy();
    select(cx, "en");
    assert!(!untransformed(cx, handle, &copy).is_empty(), "the check sees English copy in English");
    select(cx, "en-XA");
    let workspace = workspace(cx, &shell);
    let mut screens: Vec<(String, Vec<String>)> = Vec::new();
    screens.push(("conversation".into(), untransformed(cx, handle, &copy)));
    for (name, route) in [("contacts", WorkspaceRoute::Contacts), ("tasks", WorkspaceRoute::Tasks)]
    {
        let workspace = workspace.clone();
        in_window(cx, handle, move |window, cx| {
            workspace.update(cx, |w, cx| w.navigate(route, window, cx));
        });
        cx.run_until_parked();
        screens.push((name.into(), untransformed(cx, handle, &copy)));
    }
    for section in SettingsSection::ALL {
        cx.update(|cx| shell.update(cx, |s, cx| s.open_settings_at(section, cx)));
        cx.run_until_parked();
        screens.push((format!("settings {section:?}"), untransformed(cx, handle, &copy)));
        // The same screen in right to left renders too.
        select(cx, "ar-XB");
        select(cx, "en-XA");
    }
    let bypassing: Vec<_> = screens.into_iter().filter(|(_, found)| !found.is_empty()).collect();
    assert!(bypassing.is_empty(), "English text not from the catalogue: {bypassing:#?}");

    select(cx, i18n::DEFAULT_LOCALE);
}
