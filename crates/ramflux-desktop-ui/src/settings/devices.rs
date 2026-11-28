// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Devices: the account's devices (this device, last seen,
//! revoked), revocation through the confirm strip, and own-device sync export
//! and import.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::{Sizable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, SharedString, div};
use ramflux_desktop_core::{Device, Listing, SourceStatus, SyncDirection, UiEventKind};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::space;
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
    let export_reason = crate::gate::reason(
        context.store.read(cx).store(),
        ramflux_desktop_core::methods::DEVICE_SYNC_EXPORT,
    );
    let render_list = move |window: &mut gpui_kit::Window, cx: &mut App| -> gpui_kit::AnyElement {
        // The devices whose "Technical details" are open (view-local).
        let open_details = window.use_keyed_state("settings-devices-details", cx, |_, _| {
            std::collections::BTreeSet::<String>::new()
        });
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
                    // The title and the chip already say it.
                    SharedString::default()
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
                        .design_danger_quiet(cx)
                        .design_label(tr("settings.devices.revoke"))
                        .on_click(move |_, window, cx| {
                            sink.emit(
                                UiEventKind::RevokeDevicePressed { device_id: device_id.clone() },
                                window,
                                cx,
                            );
                        })
                });
                // A chip for this device or a revoked one, else the action.
                let chip = if device.this_device {
                    Some(super::design::chip(tr("settings.devices.this_chip"), cx))
                } else if device.revoked {
                    Some(super::design::chip(tr("settings.devices.revoked_chip"), cx))
                } else {
                    None
                };
                let action = h_flex().gap(space(2.)).children(chip).children(
                    revoke.map(|button| crate::gate::gated(button, revoke_reason.clone(), cx)),
                );
                // This SDK version provides no authenticated platform/origin
                // facts. Never infer native custody from a label or local flag.
                let trust = ramflux_desktop_core::projection::devices::trust(device, None);
                v_flex()
                    .id(domain_id("device", &device.id))
                    .test_support()
                    .min_w_0()
                    .child(super::design::row(
                        // A person's name for the device; its id is only in the
                        // disclosure below.
                        tr(if device.this_device {
                            "settings.devices.this_device"
                        } else {
                            "settings.devices.other_device"
                        }),
                        detail,
                        Some(action.into_any_element()),
                        cx,
                    ))
                    .child({
                        let key = device.id.to_string();
                        let open = open_details.read(cx).contains(&key);
                        let toggle_state = open_details.clone();
                        let toggle_key = key.clone();
                        v_flex()
                            .id(domain_id("device-technical", &device.id))
                            .test_support()
                            .gap(space(1.))
                            .child(
                                h_flex().child(
                                    Button::new(domain_id("device-details-toggle", &device.id))
                                        .ghost()
                                        .small()
                                        .icon(if open {
                                            gpui_kit::component::IconName::ChevronDown
                                        } else {
                                            ramflux_desktop_navigation::direction::forward_icon()
                                        })
                                        .label(tr("node.details"))
                                        .on_click(move |_, window, cx| {
                                            toggle_state.update(cx, |set, cx| {
                                                if !set.remove(&toggle_key) {
                                                    set.insert(toggle_key.clone());
                                                }
                                                cx.notify();
                                            });
                                            window.refresh();
                                        }),
                                ),
                            )
                            .when(open, |column| {
                                column.child(
                                    div()
                                        .id(domain_id("device-id", &device.id))
                                        .test_support()
                                        .text_size(gpui_kit::rems(
                                            palette(cx).size_secondary / 15.0,
                                        ))
                                        .text_color(palette(cx).secondary)
                                        .child(SharedString::from(device.id.to_string())),
                                )
                            })
                    })
                    .when(
                        trust == ramflux_desktop_core::projection::devices::DeviceTrust::Unverified
                            && !device.revoked
                            && !device.this_device,
                        |row| {
                            // The daemon lists no platform or origin facts yet:
                            // another device says so in one neutral line.
                            row.child(
                                div()
                                    .id("device-origin-note")
                                    .test_support()
                                    .role(gpui_kit::Role::Status)
                                    .aria_label(tr("settings.devices.no_details"))
                                    .pb(space(3.))
                                    .border_b_1()
                                    .border_color(palette(cx).line)
                                    .text_size(gpui_kit::rems(palette(cx).size_secondary / 15.0))
                                    .text_color(palette(cx).secondary)
                                    .child(tr("settings.devices.no_details")),
                            )
                        },
                    )
            }))
            .into_any_element()
    };
    // Design G08: "Linked devices" with the primary "Link a new device" at its
    // head (no approval contract yet: disabled with the reason), the rows,
    // then the sync section.
    let linked = SettingItem::render(move |_, window, cx: &mut App| {
        let rows = render_list(window, cx);
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
                        cx,
                    )),
            )
            .child(super::design::help(tr("settings.devices.link_unavailable"), cx))
            .child(rows)
            .into_any_element()
    });
    let sync = SettingItem::render(move |_, _, cx: &mut App| {
        let export = sink.clone();
        let buttons = h_flex().flex_wrap().gap(space(2.)).child(crate::gate::gated(
            Button::new("sync-export")
                .design_secondary(cx)
                .design_label(tr("settings.devices.export"))
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
            cx,
        ));
        super::design::section("devices-sync", tr("settings.devices.sync"), cx)
            .child(super::design::help(tr("settings.devices.sync_help"), cx))
            .child(buttons)
            .children(
                export_reason
                    .is_some()
                    .then(|| super::design::help(tr("settings.devices.link_unavailable"), cx)),
            )
            .into_any_element()
    });
    SettingPage::new(tr("settings.section.devices"))
        .group(SettingGroup::new().item(linked).item(sync))
}
