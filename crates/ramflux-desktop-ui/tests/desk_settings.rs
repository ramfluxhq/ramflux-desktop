// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings presentation on authoritative account facts and host confirmations.
mod common;
use common::*;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{Entity, TestAppContext};
use ramflux_desktop_core::{DomainEvent, Listing, SourceStatus};
use ramflux_desktop_test::load_fixture;
use ramflux_desktop_ui::{Shell, settings::SettingsSection};

fn show(cx: &mut TestAppContext, shell: &Entity<Shell>, section: SettingsSection) {
    cx.update(|cx| {
        ramflux_desktop_ui::i18n::select_locale("en", cx);
        shell.update(cx, |shell, cx| shell.open_settings_at(section, cx));
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn account_devices_backup_fit_all_widths_and_themes(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    for width in [640., 800., 1024., 1440.] {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let (handle, shell) = open_width(cx, "s09_multi_account_day", width);
            advance(cx, 1000);
            cx.update(|cx| {
                Theme::change(mode, None, cx);
                ramflux_desktop_ui::i18n::select_locale("en", cx);
            });
            for section in
                [SettingsSection::Account, SettingsSection::Devices, SettingsSection::Backup]
            {
                show(cx, &shell, section);
                in_window(cx, handle, |w, _| {
                    for id in ["settings-header", "settings-navigation", "settings-content"] {
                        let bounds = w.find(id).bounds();
                        assert!(
                            f32::from(bounds.origin.x) >= 0.
                                && f32::from(bounds.right()) <= width + 1.,
                            "{id} must fit at {width}"
                        );
                    }
                    if section == SettingsSection::Devices && width == 640. {
                        let description = w
                            .within("device-dev-laptop-01")
                            .find("device-origin-description")
                            .bounds();
                        assert!(
                            description.size.height > gpui_kit::px(40.)
                                && description.right() <= gpui_kit::px(width),
                            "the complete operator warning must wrap within the compact viewport"
                        );
                    }
                    assert!(
                        w.find("settings-close").bounds().bottom() < gpui_kit::px(860.),
                        "close remains reachable"
                    );
                });
            }
        }
    }
}

#[gpui_kit::test]
fn section_navigation_cancels_a_pending_device_hold_before_hiding_facts(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (handle, shell) = open_width(cx, "s09_multi_account_day", 640.);
    advance(cx, 1000);
    show(cx, &shell, SettingsSection::Devices);
    click_in(cx, handle, "device-dev-phone-01", "revoke");
    assert!(strip_open(cx, &shell), "source device revocation must require a host hold");
    in_window(cx, handle, |w, _| {
        let strip = w.find("hold-confirm").bounds();
        assert!(
            strip.bottom() <= gpui_kit::px(860.) && strip.size.height > gpui_kit::px(0.),
            "host hold stays in the reserved footer at compact width"
        );
    });
    click(cx, handle, "settings-section-account");
    assert!(
        !strip_open(cx, &shell),
        "pointer section navigation must cancel before hiding the reviewed device"
    );
    in_window(cx, handle, |w, cx| {
        let focused: Vec<_> = w
            .elements()
            .iter()
            .filter(|e| e.focused() == Some(true))
            .map(|e| (e.path().to_vec(), e.label().map(str::to_owned)))
            .collect();
        assert_eq!(
            w.find("settings-section-account").focused(),
            Some(true),
            "a pressed section button keeps keyboard focus: {focused:?}"
        );
        w.press("ctrl-tab", cx);
    });
    assert_eq!(
        cx.update(|cx| shell.read(cx).settings().read(cx).section()),
        SettingsSection::Backup,
        "keyboard navigation follows the same authoritative section selected by pointer"
    );
    assert!(effects(cx, &shell).is_empty(), "navigation cannot authorize revocation");
    click(cx, handle, "settings-section-devices");
    click_in(cx, handle, "device-dev-phone-01", "revoke");
    hold_confirm(cx, handle);
    assert_eq!(
        last_body(cx, &shell, ramflux_desktop_core::methods::DEVICE_REVOKE)
            .get("device_id")
            .and_then(|id| id.as_str()),
        Some("dev-phone-01"),
        "the new hold authorizes only its reviewed source device"
    );
    in_window(cx, handle, |w, _| {
        assert_eq!(
            w.find("settings").focused(),
            Some(true),
            "a completed settings hold must return focus to its host rather than the obscured workspace"
        )
    });
}

#[gpui_kit::test]
fn device_directory_states_follow_source_and_unknown_origin_stays_cautious(
    cx: &mut TestAppContext,
) {
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s09_multi_account_day");
    let mut scenario = load_fixture("s09_multi_account_day").expect("test: source scenario");
    let later_device = scenario
        .timeline
        .iter()
        .find_map(|(_, event)| match event {
            DomainEvent::DeviceUpserted(device) if device.id.as_str() == "dev-phone-01" => {
                Some(device.clone())
            }
            _ => None,
        })
        .expect("test: the scenario lists the phone");
    scenario.timeline.retain(|(_, event)| !matches!(event, DomainEvent::DeviceUpserted(_)));
    scenario.timeline.push((5000, DomainEvent::DeviceUpserted(later_device)));
    scenario.timeline.push((2000, DomainEvent::ListingsLoaded(vec![Listing::Devices])));
    scenario.timeline.push((3000, DomainEvent::SourceStatusChanged(SourceStatus::Disconnected)));
    scenario.timeline.push((4000, DomainEvent::SourceStatusChanged(SourceStatus::Connected)));
    scenario.timeline.push((
        4000,
        DomainEvent::EffectFailed(ramflux_desktop_core::EffectFailure {
            method: "device.list".into(),
            code: ramflux_desktop_core::FailureCode::Transport,
            message: "Device directory disconnected".into(),
        }),
    ));
    scenario.timeline.sort_by_key(|(at, _)| *at);
    let store = store_handle(cx, &shell);
    cx.update(|cx| store.update(cx, |s, cx| s.load_scenario(&scenario, cx)));
    advance(cx, 1000);
    show(cx, &shell, SettingsSection::Devices);
    let label =
        in_window(cx, handle, |w, _| w.find("settings-devices-state").label().map(str::to_owned));
    assert_eq!(
        label.as_deref(),
        Some("Loading devices. Waiting for the account's device directory."),
        "no source response means loading, not empty"
    );
    advance(cx, 1000);
    assert!(
        exists(cx, handle, "settings-devices-state"),
        "an empty completed directory has a rendered state"
    );
    assert!(
        !has_label(cx, handle, "Loading devices. Waiting for the account's device directory."),
        "completion ends loading"
    );
    advance(cx, 1000);
    assert!(exists(cx, handle, "settings-offline"), "disconnect exposes cached-data status");
    advance(cx, 1000);
    assert!(
        exists(cx, handle, "settings-last-error"),
        "source failures expose the last settings error"
    );
    assert!(
        has_label(cx, handle, "Last device directory error. Device directory disconnected"),
        "a failed directory has an error state rather than an empty list"
    );
    advance(cx, 1000);
    assert!(
        !exists(cx, handle, "settings-last-error"),
        "a later successful directory answer clears the stale error banner"
    );
    assert!(
        exists(cx, handle, "device-dev-phone-01"),
        "the answered directory replaces the error state"
    );
    let mut scenario = load_fixture("s09_multi_account_day").expect("test: populated source");
    let foreign = serde_json::from_value(serde_json::json!({"id":"foreign-device","account_id":"other-account","label":"Foreign device"})).expect("test: foreign source device");
    scenario.timeline.push((0, DomainEvent::DeviceUpserted(foreign)));
    scenario.timeline.sort_by_key(|(at, _)| *at);
    cx.update(|cx| store.update(cx, |s, cx| s.load_scenario(&scenario, cx)));
    advance(cx, 1000);
    show(cx, &shell, SettingsSection::Devices);
    assert!(
        exists_in(cx, handle, "device-dev-phone-01", "device-origin-warning"),
        "a source device without platform facts must retain a cautious warning"
    );
    assert!(
        !exists(cx, handle, "device-foreign-device"),
        "device rows must stay within the active account"
    );
    // The empty source layout keeps the unavailable linking control onscreen.
    let mut empty = scenario.clone();
    empty.timeline.retain(|(_, event)| !matches!(event, DomainEvent::DeviceUpserted(_)));
    cx.update(|cx| store.update(cx, |s, cx| s.load_scenario(&empty, cx)));
    advance(cx, 1000);
    show(cx, &shell, SettingsSection::Devices);
    in_window(cx, handle, |w, _| {
        assert_eq!(
            w.find("settings-link-device").disabled(),
            Some(true),
            "unsupported pairing cannot generate a code"
        )
    });
}
