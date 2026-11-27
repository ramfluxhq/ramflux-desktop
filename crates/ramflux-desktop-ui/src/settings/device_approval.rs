// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The dialog that asks the owner to approve linking a web device (design
//! board G24): the device and its kind, the origin being linked with the
//! instruction to compare it with the browser's address bar, and the
//! warning that a web session is not protected from the node operator.
//!
//! The node sends no pairing request to approve yet
//! ([`ContractGap::DevicePairingApproval`]): the dialog is drawn for a
//! request the page is given, and Approve stays disabled with the contract
//! reason until the gap closes. Reject always closes it.

use crate::i18n::tr;
use crate::modal::{self, Modal};
use crate::style::DesignButton as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, FocusHandle, SharedString, Window, div};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};
use ramflux_desktop_sdk::business::ContractGap;

/// A web device asking to be linked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRequest {
    /// The device as the browser names itself, for example "Chrome · macOS".
    pub device: SharedString,
    /// The origin the web session runs on.
    pub origin: SharedString,
}

/// One label and value row of the request.
fn fact(id: &'static str, label: SharedString, value: SharedString, cx: &App) -> AnyElement {
    let theme = cx.theme();
    h_flex()
        .id(id)
        .py(space(2.))
        .gap(space(3.))
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .w(gpui_kit::rems(172.0 / 15.0))
                .text_size(text_secondary())
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .child(div().flex_1().text_size(text_secondary()).child(value))
        .into_any_element()
}

/// The dialog for `request`; `reject` closes it.
pub fn dialog(
    request: &LinkRequest,
    focus: &FocusHandle,
    reject: impl Fn(&mut Window, &mut App) + Clone + 'static,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let palette = ramflux_desktop_navigation::palette(cx);
    let body = v_flex()
        .id("device-approval")
        .test_support()
        .gap(space(3.))
        .child(
            v_flex()
                .child(fact(
                    "device-approval-device",
                    tr("settings.devices.approve.device"),
                    request.device.clone(),
                    cx,
                ))
                .child(fact(
                    "device-approval-kind",
                    tr("settings.devices.approve.kind"),
                    tr("settings.devices.approve.kind_web"),
                    cx,
                )),
        )
        .child(
            h_flex()
                .id("device-approval-origin")
                .gap(space(4.))
                .p(space(4.))
                .items_center()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .child(Icon::new(IconName::Globe).small())
                .child(
                    v_flex()
                        .gap(space(1.))
                        .child(
                            div()
                                .text_size(text_caption())
                                .text_color(theme.muted_foreground)
                                .child(tr("settings.devices.approve.origin")),
                        )
                        .child(
                            div()
                                .text_size(text_secondary())
                                .font_medium()
                                .child(request.origin.clone()),
                        )
                        .child(
                            div()
                                .text_size(text_caption())
                                .text_color(theme.muted_foreground)
                                .child(tr("settings.devices.approve.origin_help")),
                        ),
                ),
        )
        .child(
            h_flex()
                .id("device-approval-warning")
                .gap(space(3.))
                .p(space(3.))
                .rounded(theme.radius)
                .bg(palette.warning_bg)
                .child(Icon::new(IconName::TriangleAlert).small())
                .child(
                    v_flex()
                        .gap(space(1.))
                        .child(
                            div()
                                .text_size(text_secondary())
                                .font_medium()
                                .child(tr("settings.devices.approve.warning_title")),
                        )
                        .child(
                            div()
                                .text_size(text_caption())
                                .child(tr("settings.devices.approve.warning_body")),
                        ),
                ),
        );
    let close = reject.clone();
    Modal::new("device-approval-dialog", tr("settings.devices.approve.title"), focus, close)
        .width(modal::width::NARROW)
        .closable(false)
        .child(body)
        .button(modal::cancel_button(
            "device-approval-reject",
            tr("settings.devices.approve.reject"),
            cx,
            reject,
        ))
        .button(crate::gate::gated(
            Button::new("device-approval-approve")
                .design_primary(cx)
                .design_label(tr("settings.devices.approve.approve")),
            Some(super::presentation::contract_reason(ContractGap::DevicePairingApproval)),
            cx,
        ))
        .render(cx)
}
