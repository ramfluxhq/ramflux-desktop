// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Group rename lines: the group's signed name changed (GRP-SYNC-01). One
//! compact line in the timeline with the old name, the new one and the time
//! this device learned of it. It names no author: the signed state names the
//! group's writer, not who asked for the change.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, SharedString, div};
use ramflux_desktop_core::GroupRenamePayload;

use super::MomentRow;
use crate::i18n::{self, tr_args};

/// The sentence of a group rename line.
pub fn sentence(payload: &GroupRenamePayload) -> SharedString {
    tr_args("moment.group_rename.line", &[("old", &payload.old_name), ("new", &payload.new_name)])
}

/// Renders a group rename line, with its time (it has no row header).
pub fn render(payload: &GroupRenamePayload, row: &MomentRow, cx: &App) -> impl IntoElement {
    let text = sentence(payload);
    h_flex()
        .id("group-rename")
        .aria_label(text.clone())
        .flex_wrap()
        .gap_2()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(gpui_kit::assets::IconName::Users).xsmall())
        .child(div().min_w_0().text_color(cx.theme().foreground).child(text))
        .child(div().child(i18n::time_of_day(row.moment.at)))
        .test_support()
}
