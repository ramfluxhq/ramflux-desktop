// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The fail-closed state of a grant, shown wherever the grant is decided
//! (the intervention, an approval block, the approvals dock) instead of
//! Approve / Deny: what was refused, why, and what the user can do. The
//! client never sends the decision again.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, SharedString, div};
use ramflux_desktop_core::GrantFailure;
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};

use crate::i18n::tr;

/// Why a grant failed, in plain words.
pub fn reason(failure: GrantFailure) -> SharedString {
    tr(match failure {
        GrantFailure::AlreadyUsed => "grant.failure.already_used",
        GrantFailure::Expired => "grant.failure.expired",
        GrantFailure::ArgumentsMismatch => "grant.failure.arguments_mismatch",
        GrantFailure::Replay => "grant.failure.replay",
        GrantFailure::SignatureRejected => "grant.failure.signature_rejected",
        GrantFailure::SignedApprovalRequired => "grant.failure.signed_approval_required",
        GrantFailure::Invalidated => "grant.failure.invalidated",
    })
}

/// The inline failure state.
pub fn render(failure: GrantFailure, cx: &App) -> AnyElement {
    h_flex()
        .id("grant-failure")
        .gap(space(2.))
        .items_start()
        .p(space(2.))
        .border_1()
        .border_color(cx.theme().danger)
        .rounded(cx.theme().radius)
        .child(Icon::new(IconName::CircleX).small().text_color(cx.theme().danger))
        .child(
            v_flex()
                .min_w_0()
                .gap(space(1.))
                .child(
                    div()
                        .text_size(text_secondary())
                        .font_medium()
                        .child(tr("grant.failure.title")),
                )
                .child(div().text_size(text_caption()).child(reason(failure)))
                .child(
                    div()
                        .text_size(text_caption())
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("grant.failure.next")),
                ),
        )
        .test_support()
        .into_any_element()
}
