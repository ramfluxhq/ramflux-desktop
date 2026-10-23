// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Devices: the account's devices (this device, last seen,
//! revoked), revocation through the hold strip, and own-device sync export
//! and import.

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, div};
use ramflux_desktop_core::{Device, SyncDirection, UiEventKind};

use super::{SectionContext, SettingsSection, note};
use crate::i18n::{self, tr, tr_args};
use crate::ids::domain_id;

/// Sync bundle path offered until a native file dialog is wired.
pub const SYNC_PATH: &str = "~/Documents/ramflux-device-sync.bundle";

/// The devices section.
pub fn page(context: &SectionContext, cx: &App) -> SettingPage {
    let devices: Vec<Device> = context.store.read(cx).store().devices().cloned().collect();
    let sink = context.sink(SettingsSection::Devices);
    let list_sink = sink.clone();
    let list = SettingItem::render(move |_, _, cx: &mut App| {
        if devices.is_empty() {
            return note("settings.devices.none", cx);
        }
        v_flex()
            .id("settings-devices")
            .gap_1()
            .children(devices.iter().map(|device| {
                let detail = if device.revoked {
                    tr("settings.devices.revoked")
                } else if device.this_device {
                    tr("settings.devices.this")
                } else {
                    device.last_seen.map_or_else(
                        || tr("settings.devices.never"),
                        |at| {
                            tr_args("settings.devices.last_seen", &[("time", &i18n::date_time(at))])
                        },
                    )
                };
                let revoke = (!device.revoked && !device.this_device).then(|| {
                    let sink = list_sink.clone();
                    let device_id = device.id.clone();
                    Button::new("revoke")
                        .xsmall()
                        .outline()
                        .label(tr("settings.devices.revoke"))
                        .on_click(move |_, window, cx| {
                            sink.emit(
                                UiEventKind::RevokeDevicePressed { device_id: device_id.clone() },
                                window,
                                cx,
                            );
                        })
                });
                h_flex()
                    .id(domain_id("device", &device.id))
                    .gap_2()
                    .py_1()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().font_medium().child(device.label.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(detail),
                            ),
                    )
                    .children(revoke)
            }))
            .into_any_element()
    });
    let sync = SettingItem::render(move |_, _, _: &mut App| {
        let export = sink.clone();
        let import = sink.clone();
        h_flex()
            .gap_2()
            .child(
                Button::new("sync-export")
                    .small()
                    .outline()
                    .label(tr("settings.devices.export"))
                    .on_click(move |_, window, cx| {
                        export.emit(
                            UiEventKind::DeviceSyncRequested {
                                direction: SyncDirection::Export,
                                path: SYNC_PATH.to_owned(),
                            },
                            window,
                            cx,
                        );
                    }),
            )
            .child(
                Button::new("sync-import")
                    .small()
                    .outline()
                    .label(tr("settings.devices.import"))
                    .on_click(move |_, window, cx| {
                        import.emit(
                            UiEventKind::DeviceSyncRequested {
                                direction: SyncDirection::Import,
                                path: SYNC_PATH.to_owned(),
                            },
                            window,
                            cx,
                        );
                    }),
            )
            .into_any_element()
    });
    SettingPage::new(tr("settings.section.devices"))
        .group(SettingGroup::new().item(list))
        .group(SettingGroup::new().title(tr("settings.devices.sync")).item(sync))
}
