// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Devices: the account's devices (this device, last seen,
//! revoked), revocation through the confirm strip, and own-device sync export
//! and import.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, SharedString, div};
use ramflux_desktop_core::{Device, Listing, SourceStatus, SyncDirection, UiEventKind};
use ramflux_desktop_navigation::{generated::catalog, palette};

use super::{SectionContext, SettingsSection, presentation};
use crate::file_dialog::{self, StartDir};
use crate::i18n::{self, tr, tr_args};
use crate::ids::domain_id;
use crate::style::DesignButton as _;

/// File name suggested for an exported sync bundle.
pub const SYNC_FILE_NAME: &str = "ramflux-device-sync.bundle";

/// The devices section.
pub(crate) fn page(
    context: &SectionContext,
    errors: &super::errors::ErrorScope,
    cx: &App,
) -> SettingPage {
    let account = super::account::active_account(context, cx).map(|account| account.id);
    let rendered = context.store.read(cx).active_account().cloned();
    let store = context.store.read(cx).store();
    let devices: Vec<Device> = store
        .devices()
        .filter(|device| account.as_ref() == Some(&device.account_id))
        .cloned()
        .collect();
    // Listed per account: one account's answer never ends another's loading.
    let listed = errors.devices_listed(rendered.as_ref()) && store.is_listed(Listing::Devices);
    let offline = store.source_status() == SourceStatus::Disconnected;
    // Relative times ("5 minutes ago") are read against the session clock.
    let now = context.store.read(cx).session().map(crate::app::AccountSession::now);
    // Only a directory failure newer than the last success, switch or
    // reconnect for this account explains an empty list.
    let failure = errors
        .current_method(
            rendered.as_ref(),
            SettingsSection::Devices,
            super::errors::DEVICE_LIST,
            store,
        )
        .map(|failure| i18n::backend_text(&failure.message));
    let revoke_reason = crate::gate::reason(store, ramflux_desktop_core::methods::DEVICE_REVOKE);
    let sink = context.sink(SettingsSection::Devices);
    let list_sink = sink.clone();
    let (export_reason, import_reason) = {
        let store = context.store.read(cx).store();
        (
            crate::gate::reason(store, ramflux_desktop_core::methods::DEVICE_SYNC_EXPORT),
            crate::gate::reason(store, ramflux_desktop_core::methods::DEVICE_SYNC_IMPORT),
        )
    };
    let render_list = move |cx: &mut App| -> gpui_kit::AnyElement {
        if devices.is_empty() {
            let (title, description, kind) = if let Some(reason) = failure.as_ref() {
                (tr("settings_ui.devices_error"), reason.clone(), catalog::StateKind::Error)
            } else if offline {
                (
                    tr("settings_ui.offline"),
                    tr("settings_ui.offline_help"),
                    catalog::StateKind::Offline,
                )
            } else if listed {
                (
                    tr("settings.devices.none"),
                    tr("settings_ui.devices_empty_help"),
                    catalog::StateKind::Empty,
                )
            } else {
                (
                    tr("settings_ui.devices_loading"),
                    tr("settings_ui.devices_loading_help"),
                    catalog::StateKind::Loading,
                )
            };
            return presentation::notice("settings-devices-state", title, description, kind, cx);
        }
        v_flex()
            .id("settings-devices")
            .test_support()
            .min_w_0()
            .children(devices.iter().map(|device| {
                let detail = if device.revoked {
                    tr("settings.devices.revoked")
                } else if device.this_device {
                    tr("settings.devices.this")
                } else {
                    device.last_seen.map_or_else(
                        || tr("settings.devices.never"),
                        |at| {
                            let time = now.map_or_else(
                                || i18n::date_time(at),
                                |now| i18n::relative(at, now).to_string(),
                            );
                            tr_args("settings.devices.last_seen", &[("time", &time)])
                        },
                    )
                };
                let revoke = (!device.revoked && !device.this_device).then(|| {
                    let sink = list_sink.clone();
                    let device_id = device.id.clone();
                    // Design G08: a quiet danger button.
                    Button::new("revoke")
                        .ghost()
                        .h(gpui_kit::rems(32.0 / 15.0))
                        .px(gpui_kit::rems(12.0 / 15.0))
                        .bg(palette(cx).danger_bg)
                        .text_color(palette(cx).danger)
                        .label(tr("settings.devices.revoke"))
                        .on_click(move |_, window, cx| {
                            sink.emit(
                                UiEventKind::RevokeDevicePressed { device_id: device_id.clone() },
                                window,
                                cx,
                            );
                        })
                });
                // A chip for this device or a revoked one, else the action.
                let unverified_origin = !device.this_device
                    && !device.revoked
                    && ramflux_desktop_core::projection::devices::trust(device, None)
                        == ramflux_desktop_core::projection::devices::DeviceTrust::Unverified;
                let chip = if unverified_origin {
                    Some(crate::workspace::sections::warning_chip(
                        "device-origin-chip",
                        tr("settings.devices.unverified_chip"),
                        cx,
                    ))
                } else if device.this_device {
                    Some(super::design::chip(tr("settings.devices.this_chip"), cx))
                } else if device.revoked {
                    Some(super::design::chip(tr("settings.devices.revoked_chip"), cx))
                } else {
                    None
                };
                let action = h_flex().gap_2().children(chip).children(
                    revoke.map(|button| crate::gate::gated(button, revoke_reason.clone())),
                );
                // This SDK version provides no authenticated platform/origin
                // facts. Never infer native custody from a label or local flag.
                let trust = ramflux_desktop_core::projection::devices::trust(device, None);
                v_flex()
                    .id(domain_id("device", &device.id))
                    .test_support()
                    .min_w_0()
                    .child(super::design::row(
                        ramflux_desktop_navigation::row::list_text(&device.label),
                        i18n::join(&detail, &device.id.to_string()),
                        Some(action.into_any_element()),
                        cx,
                    ))
                    .when(
                        trust == ramflux_desktop_core::projection::devices::DeviceTrust::Unverified,
                        |row| {
                            // Revoked rows and this device carry no actionable
                            // origin risk: one compact line keeps the cautious
                            // fact without repeating the full warning.
                            if device.revoked || device.this_device {
                                row.child(presentation::compact_warning(
                                    "device-origin-note",
                                    tr("settings_ui.origin_unknown"),
                                    cx,
                                ))
                            } else {
                                // Board G08 ("Chrome · macOS"): the operator
                                // risk as the row's own wrapped line under a
                                // warning chip, not a separate block.
                                row.child(
                                    div()
                                        .id("device-origin-warning")
                                        .test_support()
                                        .role(gpui_kit::Role::Status)
                                        .aria_label(crate::i18n::sentences(
                                            &tr("settings_ui.origin_unknown"),
                                            &tr("settings_ui.origin_warning"),
                                        ))
                                        .pb_3()
                                        .border_b_1()
                                        .border_color(palette(cx).line)
                                        .child(
                                            div()
                                                .id("device-origin-description")
                                                .test_support()
                                                .whitespace_normal()
                                                .text_size(gpui_kit::rems(
                                                    palette(cx).size_secondary / 15.0,
                                                ))
                                                .text_color(palette(cx).secondary)
                                                .child(i18n::join(
                                                    &tr("settings_ui.origin_unknown"),
                                                    &tr("settings_ui.origin_warning"),
                                                )),
                                        ),
                                )
                            }
                        },
                    )
            }))
            .into_any_element()
    };
    // Design G08: "Linked devices" with the primary "Link a new device" at its
    // head (no approval contract yet: disabled with the reason), the rows,
    // then the sync section.
    let linked = SettingItem::render(move |_, _, cx: &mut App| {
        let rows = render_list(cx);
        super::design::section("devices-linked", SharedString::default(), cx)
            .child(
                h_flex()
                    .w_full()
                    .child(
                        div()
                            .flex_1()
                            .text_size(gpui_kit::rems(palette(cx).size_subheading / 15.0))
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(tr("settings.devices.linked")),
                    )
                    .child(crate::gate::gated(
                        crate::style::primary_button("settings-link-device", cx)
                            .label(tr("settings_ui.link_device")),
                        Some(presentation::contract_reason(
                            ramflux_desktop_sdk::business::ContractGap::DevicePairingApproval,
                        )),
                    )),
            )
            .child(rows)
            .into_any_element()
    });
    let sync = SettingItem::render(move |_, _, cx: &mut App| {
        let export = sink.clone();
        let import = sink.clone();
        let buttons = h_flex()
            .flex_wrap()
            .gap_2()
            .child(crate::gate::gated(
                Button::new("sync-export")
                    .design_secondary(cx)
                    .label(tr("settings.devices.export"))
                    .on_click(move |_, window, cx| {
                        let export = export.clone();
                        file_dialog::save_as(
                            StartDir::Documents,
                            SYNC_FILE_NAME,
                            window,
                            cx,
                            move |path, window, cx| {
                                export.emit(
                                    UiEventKind::DeviceSyncRequested {
                                        direction: SyncDirection::Export,
                                        path,
                                    },
                                    window,
                                    cx,
                                );
                            },
                        );
                    }),
                export_reason.clone(),
            ))
            .child(crate::gate::gated(
                Button::new("sync-import")
                    .design_secondary(cx)
                    .label(tr("settings.devices.import"))
                    .on_click(move |_, window, cx| {
                        let import = import.clone();
                        file_dialog::open_file(
                            tr("settings.devices.import"),
                            window,
                            cx,
                            move |path, window, cx| {
                                import.emit(
                                    UiEventKind::DeviceSyncRequested {
                                        direction: SyncDirection::Import,
                                        path,
                                    },
                                    window,
                                    cx,
                                );
                            },
                        );
                    }),
                import_reason.clone(),
            ));
        super::design::section("devices-sync", tr("settings.devices.sync"), cx)
            .child(super::design::help(tr("settings.devices.sync_help"), cx))
            .child(buttons)
            .into_any_element()
    });
    SettingPage::new(tr("settings.section.devices"))
        .group(SettingGroup::new().item(linked).item(sync))
}
