// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Identity and security service lines (design C19): a centred, quiet line
//! in the timeline when a contact's identity changed, when the user
//! accepted the new identity, or when the encryption was re-established
//! after a fork. They carry no action: reviewing and accepting a change
//! happen in the identity change sheet, opened from the header banner.

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, SharedString, div};
use ramflux_desktop_core::{IdentityChangeKind, IdentityChangePayload, SecurityNoticePayload};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption};

use super::RowRefs;
use crate::i18n::{tr, tr_args};

/// One centred service line with an icon.
fn line(
    id: &'static str,
    icon: IconName,
    changed: bool,
    text: SharedString,
    cx: &App,
) -> impl IntoElement {
    let color = if changed { cx.theme().warning } else { cx.theme().muted_foreground };
    h_flex()
        .id(id)
        .aria_label(text.clone())
        .w_full()
        .justify_center()
        .gap(space(2.))
        .text_size(text_caption())
        .text_color(color)
        .child(Icon::new(icon).xsmall())
        .child(div().min_w_0().child(text))
        .test_support()
}

/// Renders an identity change line: "<name>'s identity changed" or "You
/// accepted <name>'s new identity".
pub fn render(payload: &IdentityChangePayload, refs: &RowRefs, cx: &App) -> impl IntoElement {
    let name = refs.name(&payload.participant_id);
    match payload.change {
        IdentityChangeKind::Changed => line(
            "identity-changed",
            IconName::TriangleAlert,
            true,
            tr_args("identity.timeline.changed", &[("name", &name)]),
            cx,
        ),
        IdentityChangeKind::Accepted => line(
            "identity-accepted",
            IconName::CircleCheck,
            false,
            tr_args("identity.timeline.accepted", &[("name", &name)]),
            cx,
        ),
    }
}

/// Renders a security notice line: the encryption was re-established.
pub fn notice(payload: &SecurityNoticePayload, cx: &App) -> impl IntoElement {
    match payload.warning {
        ramflux_desktop_core::SecurityWarning::ForkRecovered => line(
            "security-fork-recovered",
            IconName::ShieldAlert,
            true,
            tr("security.warning.fork_recovered"),
            cx,
        ),
    }
}
