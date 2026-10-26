// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Name change lines: a participant's public name changed through its
//! root-signed profile (an authenticated rename). One compact line in the
//! timeline, by the participant: the old name, the new one, and the
//! principal id, so the new name cannot pass for someone else's.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, SharedString, div};
use ramflux_desktop_core::NameChangePayload;

use super::MomentRow;
use crate::i18n::{self, tr_args};

/// The sentence of a name change line.
pub fn sentence(payload: &NameChangePayload) -> SharedString {
    tr_args("moment.name_change.line", &[("old", &payload.old_name), ("new", &payload.new_name)])
}

/// Renders a name change line, with the principal id and its time (it has
/// no row header).
pub fn render(payload: &NameChangePayload, row: &MomentRow, cx: &App) -> impl IntoElement {
    let text = sentence(payload);
    h_flex()
        .id("name-change")
        .aria_label(tr_args(
            "moment.name_change.a11y",
            &[("line", &text), ("id", payload.participant_id.as_str())],
        ))
        .gap_2()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(gpui_kit::assets::IconName::UserPen).xsmall())
        .child(div().min_w_0().text_color(cx.theme().foreground).child(text))
        .child(
            div()
                .font_family(cx.theme().mono_font_family.clone())
                .child(payload.participant_id.short()),
        )
        .child(div().child(i18n::time_of_day(row.moment.at)))
        .test_support()
}
