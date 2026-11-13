// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Generated presentation slots around existing settings behavior owners.
use crate::i18n::tr;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName};
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
        .child(div().text_sm().text_color(cx.theme().muted_foreground).child(label))
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
        .aria_label(format!("{title}. {description}"))
        .min_w_0()
        .w_full()
        .whitespace_normal()
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

pub(super) fn warning(
    id: impl Into<ElementId>,
    title: SharedString,
    description: SharedString,
    cx: &App,
) -> AnyElement {
    let theme = palette(cx);
    gpui_kit::component::v_flex()
        .id(id)
        .test_support()
        .role(Role::Status)
        .aria_label(format!("{title}. {description}"))
        .min_w_0()
        .w_full()
        .whitespace_normal()
        .bg(theme.warning_bg)
        .rounded(gpui_kit::rems(theme.radius_md / 16.0))
        // The generated banner's description lives in a row with intrinsic
        // minimum width. Keep its chrome, and constrain the long host copy in
        // a separate full-width block so every risk fact remains readable.
        .child(catalog::trust_banner(
            catalog::NoticeContent::new(
                title,
                SharedString::default(),
                Icon::new(IconName::TriangleAlert).text_color(cx.theme().warning),
                div(),
            ),
            false,
            theme,
        ))
        .child(
            div()
                .id("device-origin-description")
                .test_support()
                .min_w_0()
                .w_full()
                .px_3()
                .pb_3()
                .text_sm()
                .text_color(theme.text)
                .whitespace_normal()
                .child(description),
        )
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
