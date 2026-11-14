// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Generated presentation slots around existing settings behavior owners.
use crate::i18n::tr;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ElementId, IntoElement, Role, SharedString, TestSupportExt as _, div,
};
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
        .gap_2()
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
        // Design `RF / Tip bar`: the notice sits on the surface fill.
        .p_3()
        .rounded(gpui_kit::rems(palette(cx).radius_md / 15.0))
        .bg(palette(cx).surface)
        .child(catalog::state_block(
            "settings-state".into(),
            title,
            description,
            kind,
            div(),
            palette(cx),
        ))
        .into_any_element()
}

/// One muted line with a warning icon: a cautious fact with no action.
pub(super) fn compact_warning(
    id: impl Into<ElementId>,
    title: SharedString,
    cx: &App,
) -> AnyElement {
    gpui_kit::component::h_flex()
        .id(id)
        .test_support()
        .role(Role::Status)
        .aria_label(title.clone())
        .gap_2()
        .min_w_0()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(IconName::TriangleAlert).xsmall().text_color(cx.theme().warning))
        .child(div().min_w_0().truncate().child(title))
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
