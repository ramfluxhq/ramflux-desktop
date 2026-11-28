// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Generated presentation slots around existing settings behavior owners.
use crate::i18n::tr;
use gpui_kit::component::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ElementId, IntoElement, Role, SharedString, TestSupportExt as _, div,
};
use ramflux_desktop_navigation::scale::space;
use ramflux_desktop_navigation::{generated::catalog, palette};

pub(super) fn contract_reason(gap: ramflux_desktop_sdk::business::ContractGap) -> SharedString {
    use ramflux_desktop_sdk::business::ContractGap;
    tr(match gap {
        ContractGap::DevicePairingApproval => "settings_ui.pairing_unavailable",
        ContractGap::AccountPrivacy => "settings_ui.privacy_unavailable",
        _ => "settings_ui.source_unavailable",
    })
}

pub(crate) fn field(label: SharedString, editor: impl IntoElement, cx: &App) -> AnyElement {
    // Keep the host's masked editor, IDs and one-shot secret handoff intact.
    gpui_kit::component::v_flex()
        .gap(space(2.))
        .min_w_0()
        // Design: a 13 px medium label above the editor.
        .child(
            div()
                .text_size(gpui_kit::rems(palette(cx).size_secondary / 15.0))
                .font_weight(gpui_kit::FontWeight::MEDIUM)
                .text_color(palette(cx).text)
                .child(label),
        )
        .child(editor)
        .into_any_element()
}

pub(super) fn notice(
    id: impl Into<ElementId>,
    title: SharedString,
    description: SharedString,
    kind: catalog::StateKind,
    cx: &App,
) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .role(if kind == catalog::StateKind::Error { Role::Alert } else { Role::Status })
        .aria_label(crate::i18n::sentences(&title, &description))
        .min_w_0()
        .w_full()
        .whitespace_normal()
        // Design `RF / Tip bar`: the tone's icon and one line of text
        // (title, then description), danger for an error.
        .child(crate::status::tip_bar(
            if kind == catalog::StateKind::Error {
                crate::status::Tone::Danger
            } else {
                crate::status::Tone::Neutral
            },
            (kind == catalog::StateKind::Loading).then_some(IconName::LoaderCircle),
            crate::i18n::sentences(&title, &description),
            cx,
        ))
        .into_any_element()
}

pub(super) fn offline(cx: &App) -> AnyElement {
    notice(
        "settings-offline",
        tr("settings_ui.offline"),
        tr("settings_ui.offline_help"),
        catalog::StateKind::Offline,
        cx,
    )
}
