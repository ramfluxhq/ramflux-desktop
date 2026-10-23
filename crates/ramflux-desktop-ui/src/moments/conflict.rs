// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Conflict moments: "this update does not match your local version",
//! explained in plain words, a side-by-side compare of the two versions
//! (read-only `Diff`, state owned by the conversation field), and the three
//! resolutions while the conflict is open.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::diff::Diff;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, div, rems};
use ramflux_desktop_core::{
    BlockId, ConflictPayload, ConflictResolution, ConflictSide, Lifecycle, Scope, UiEventKind,
};

use super::{RowRefs, RowState};
use crate::bubble::EventSink;
use crate::i18n::{tr, tr_args};

/// Height of the compare view in rems; the diff virtualizes inside it.
const COMPARE_HEIGHT: f32 = 9.;

/// The unified patch the compare view shows: local on the left, remote on
/// the right.
pub fn compare_patch(name: &str, local: &ConflictSide, remote: &ConflictSide) -> String {
    let left: &[String] = &local.lines;
    let right: &[String] = &remote.lines;
    let mut patch = format!(
        "--- a/{name} ({})\n+++ b/{name} ({})\n@@ -1,{} +1,{} @@\n",
        local.version,
        remote.version,
        left.len(),
        right.len()
    );
    let common = left.iter().zip(right).take_while(|(a, b)| a == b).count();
    let tail = left[common..]
        .iter()
        .rev()
        .zip(right[common..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    for line in &left[..common] {
        patch.push_str(&format!(" {line}\n"));
    }
    for line in &left[common..left.len() - tail] {
        patch.push_str(&format!("-{line}\n"));
    }
    for line in &right[common..right.len() - tail] {
        patch.push_str(&format!("+{line}\n"));
    }
    for line in &left[left.len() - tail..] {
        patch.push_str(&format!(" {line}\n"));
    }
    patch
}

/// Renders a conflict.
pub fn render(
    payload: &ConflictPayload,
    refs: &RowRefs,
    lifecycle: Lifecycle,
    sink: &EventSink,
    state: &RowState,
    cx: &App,
) -> impl IntoElement {
    let name = refs
        .objects
        .get(&payload.object_id)
        .map_or_else(|| payload.object_id.to_string(), |o| o.name.clone());
    let open = payload.resolution.is_none() && lifecycle == Lifecycle::NeedsMe;
    let choice_sink = sink.scoped(Scope::Block(BlockId::new("resolution")));
    let side = |label: &'static str, side: &ConflictSide| {
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .p_2()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr_args(label, &[("version", &side.version)])),
            )
            .child(div().text_sm().child(side.summary.clone()))
    };
    let choices = [
        ("conflict-keep-local", "moment.conflict.keep_local", ConflictResolution::KeepLocal),
        ("conflict-keep-remote", "moment.conflict.keep_remote", ConflictResolution::KeepRemote),
        ("conflict-keep-both", "moment.conflict.keep_both", ConflictResolution::KeepBoth),
    ];
    let buttons = choices.into_iter().map(|(id, key, resolution)| {
        let sink = choice_sink.clone();
        Button::new(id).small().outline().label(tr(key)).on_click(move |_, window, cx| {
            sink.emit(UiEventKind::ConflictResolved { resolution }, window, cx);
        })
    });
    let chosen = payload.resolution.map(|resolution| {
        tr(match resolution {
            ConflictResolution::KeepLocal => "moment.conflict.kept_local",
            ConflictResolution::KeepRemote => "moment.conflict.kept_remote",
            ConflictResolution::KeepBoth => "moment.conflict.kept_both",
        })
    });
    v_flex()
        .id("conflict")
        .gap_2()
        .p_3()
        .border_1()
        .border_color(if open { cx.theme().warning } else { cx.theme().border })
        .rounded(cx.theme().radius_lg)
        .bg(cx.theme().group_box)
        .child(
            h_flex()
                .gap_2()
                .child(Icon::new(IconName::Replace).small().text_color(cx.theme().warning))
                .child(
                    div()
                        .text_sm()
                        .font_medium()
                        .child(tr_args("moment.conflict.title", &[("name", &name)])),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(tr("moment.conflict.explain")),
        )
        .child(
            h_flex()
                .gap_2()
                .items_stretch()
                .child(side("moment.conflict.local", &payload.local))
                .child(side("moment.conflict.remote", &payload.remote)),
        )
        .when_some(state.diff.clone(), |col, diff| {
            col.child(
                div()
                    .id("conflict-compare")
                    .w_full()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(cx.theme().radius)
                    .overflow_hidden()
                    .child(Diff::new(&diff).h(rems(COMPARE_HEIGHT)).w_full())
                    .test_support(),
            )
        })
        .child(if open {
            h_flex().flex_wrap().gap_2().children(buttons).into_any_element()
        } else {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(chosen.unwrap_or_else(|| tr("moment.conflict.closed")))
                .into_any_element()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side(version: &str, lines: &[&str]) -> ConflictSide {
        ConflictSide {
            version: version.to_owned(),
            summary: String::new(),
            lines: lines.iter().map(|l| (*l).to_owned()).collect(),
        }
    }

    #[test]
    fn the_patch_keeps_shared_lines_as_context() {
        let patch = compare_patch(
            "ledger.ods",
            &side("v7", &["a", "b", "c", "d"]),
            &side("v8", &["a", "x", "y", "d"]),
        );
        assert!(patch.contains(" a\n-b\n-c\n+x\n+y\n d\n"), "{patch}");
        assert!(gpui_kit::component::diff::DiffFile::parse(&patch).is_ok());
    }
}
