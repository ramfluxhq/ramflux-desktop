// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A language switch at runtime leaves no Chinese on any main surface: every
//! named element (labels and values) of the message list and conversation,
//! groups, contacts, strangers, tasks, agents, the open command palette,
//! every settings section and every dock tool is read after switching from
//! Chinese to English, and none may still contain CJK text. The scenario's
//! own data is English, so any Chinese left is interface text that was kept
//! from before the switch.
//!
//! Its own test binary: the locale is process-wide.

mod common;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestQueryExt as _;
use ramflux_desktop_ui::i18n;
use ramflux_desktop_ui::settings::SettingsSection;
use ramflux_desktop_ui::workspace::{DockTool, WorkspaceRoute};

type Handle = gpui_kit::WindowHandle<gpui_kit::base::Root>;

fn cjk(text: &str) -> bool {
    text.chars().any(|c| matches!(c, '\u{3000}'..='\u{303f}' | '\u{4e00}'..='\u{9fff}' | '\u{ff00}'..='\u{ffef}'))
}

/// Language names are written in their own language (the Chinese name
/// of Chinese is shown in every locale): they are not interface text.
fn endonym(text: &str) -> bool {
    text.ends_with(i18n::tr("locale.chinese").as_ref())
}

/// Every named text of the window that still holds CJK.
fn chinese_left(cx: &mut TestAppContext, handle: Handle) -> Vec<String> {
    in_window(cx, handle, |window, _| {
        window
            .elements()
            .into_iter()
            .flat_map(|e| [e.label().map(str::to_owned), e.value().map(str::to_owned)])
            .flatten()
            .filter(|text| cjk(text) && !endonym(text))
            .collect()
    })
}

/// Builds the surface in Chinese, switches to English and checks it.
fn check(cx: &mut TestAppContext, handle: Handle, surface: &str) {
    cx.update(|cx| i18n::select_locale("zh-CN", cx));
    cx.run_until_parked();
    in_window(cx, handle, |_, _| {});
    assert!(
        !chinese_left(cx, handle).is_empty(),
        "{surface}: the surface is built in Chinese first"
    );
    cx.update(|cx| i18n::select_locale("en", cx));
    cx.run_until_parked();
    let left = chinese_left(cx, handle);
    assert!(left.is_empty(), "{surface}: Chinese kept after the switch: {left:#?}");
}

#[gpui_kit::test]
fn no_chinese_remains_on_any_main_surface_after_a_runtime_switch(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    let workspace = workspace(cx, &shell);
    let route = |cx: &mut TestAppContext, route: WorkspaceRoute| {
        let workspace = workspace.clone();
        in_window(cx, handle, move |window, cx| {
            workspace.update(cx, |w, cx| w.navigate(route, window, cx));
        });
    };
    show_context(cx, handle, &shell, "c-team");
    check(cx, handle, "messages and conversation");
    for (name, r) in [
        ("groups", WorkspaceRoute::Groups),
        ("contacts", WorkspaceRoute::Contacts),
        ("strangers", WorkspaceRoute::Strangers),
        ("tasks", WorkspaceRoute::Tasks),
        ("agents", WorkspaceRoute::Agents),
    ] {
        route(cx, r);
        check(cx, handle, name);
    }
    route(cx, WorkspaceRoute::Messages);
    show_context(cx, handle, &shell, "c-team");
    for tool in DockTool::ALL {
        let workspace = workspace.clone();
        in_window(cx, handle, move |window, cx| {
            workspace.update(cx, |w, cx| w.open_tool(tool, window, cx));
        });
        check(cx, handle, &format!("dock {tool:?}"));
    }
    in_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(ramflux_desktop_ui::actions::OpenPalette), cx)
    });
    check(cx, handle, "command palette");
    in_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(ramflux_desktop_ui::actions::ClosePalette), cx)
    });
    for section in SettingsSection::ALL {
        cx.update(|cx| shell.update(cx, |s, cx| s.open_settings_at(section, cx)));
        cx.run_until_parked();
        check(cx, handle, &format!("settings {section:?}"));
    }
    cx.update(|cx| i18n::select_locale(i18n::DEFAULT_LOCALE, cx));
}
