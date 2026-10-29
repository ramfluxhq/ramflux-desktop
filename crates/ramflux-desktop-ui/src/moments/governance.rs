// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Governance moments: membership, role and invitation records. The actions
//! themselves (kick, ban, role changes through the hold strip, invitations)
//! live in the members dock.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, div};
use ramflux_desktop_core::{GovernanceAction, GovernancePayload};

use super::RowRefs;
use crate::i18n::{self, tr_args};

/// Renders a governance record.
pub fn render(payload: &GovernancePayload, refs: &RowRefs, cx: &App) -> impl IntoElement {
    let name = refs.name(&payload.target);
    let icon = match payload.action {
        GovernanceAction::Removed | GovernanceAction::Banned | GovernanceAction::Left => {
            IconName::CircleX
        }
        GovernanceAction::RoleChanged => IconName::Settings2,
        GovernanceAction::Invited | GovernanceAction::Joined => IconName::CircleUser,
    };
    let text = tr_args(
        "moment.governance.record",
        &[("name", &name), ("action", &i18n::governance_action(payload.action))],
    );
    h_flex()
        .id("governance-record")
        .aria_label(text.clone())
        .gap_2()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(icon).small())
        .child(div().text_color(cx.theme().foreground).child(text))
        .when_some(payload.role, |row, role| {
            row.child(
                div()
                    .px_1()
                    .text_xs()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(cx.theme().radius)
                    .child(i18n::role(role)),
            )
        })
        .test_support()
}
