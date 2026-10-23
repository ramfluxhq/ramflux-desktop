// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Task progress moments: intent, then steps with their states, then the
//! result.
//!
//! Collapsed, the card shows the title, a step count and the current step;
//! expanded (view-local), every step. The step that needs the user always
//! expands with its decision inline: the grant it waits on, with Approve and
//! Deny scoped to that step (`Block(step-N)`), so the root decides exactly
//! that grant.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, IntoElement, div};
use ramflux_desktop_core::{
    BlockId, Grant, GrantState, Lifecycle, Scope, Task, TaskStep, UiEventKind,
};

use super::{RowRefs, RowState};
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args};
use crate::ids::domain_id;
use crate::root::step_block_id;

/// Renders a task progress body.
pub fn render(
    refs: &RowRefs,
    lifecycle: Lifecycle,
    sink: &EventSink,
    state: &RowState,
    cx: &App,
) -> impl IntoElement {
    let Some(task) = refs.task.clone() else {
        return v_flex()
            .id("task-progress")
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(tr("moment.task.missing"));
    };
    let done = task.steps.iter().filter(|s| s.lifecycle == Lifecycle::Done).count();
    let total = task.steps.len();
    let expanded = state.expanded;
    let steps: Vec<AnyElement> = task
        .steps
        .iter()
        .enumerate()
        .filter(|(_, step)| {
            expanded
                || matches!(
                    step.lifecycle,
                    Lifecycle::InProgress | Lifecycle::NeedsMe | Lifecycle::Failed
                )
        })
        .map(|(ix, step)| step_row(ix, step, refs, sink, cx))
        .collect();
    let disclosure = sink.scoped(Scope::Block(BlockId::new("steps")));
    let cancel = sink.scoped(Scope::Block(BlockId::new("cancel")));
    // Lossy by design: the bar is only displayed.
    #[allow(clippy::cast_precision_loss)]
    let percent = if total == 0 { 0.0 } else { done as f32 / total as f32 * 100.0 };
    v_flex()
        .id("task-progress")
        .gap_2()
        .p_3()
        .border_1()
        .border_color(if lifecycle == Lifecycle::NeedsMe {
            cx.theme().warning
        } else {
            cx.theme().border
        })
        .rounded(cx.theme().radius_lg)
        .bg(cx.theme().group_box)
        .child(
            h_flex()
                .gap_2()
                .child(
                    Icon::new(IconName::LayoutDashboard)
                        .small()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(div().flex_1().min_w_0().text_sm().font_medium().child(task.title.clone()))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(tr_args(
                    "moment.task.count",
                    &[("done", &done.to_string()), ("total", &total.to_string())],
                ))),
        )
        .child(Progress::new("task-bar").value(percent).accessibility_label(task.title.clone()))
        .children(steps)
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("task-steps")
                        .ghost()
                        .xsmall()
                        .icon(if expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                        .label(tr(if expanded {
                            "moment.task.hide_steps"
                        } else {
                            "moment.task.show_steps"
                        }))
                        .on_click(move |_, window, cx| {
                            disclosure.emit(
                                UiEventKind::ExpandToggled { expanded: !expanded },
                                window,
                                cx,
                            );
                        }),
                )
                .when(!task.lifecycle.is_terminal(), |row| {
                    row.child(
                        Button::new("task-cancel")
                            .ghost()
                            .xsmall()
                            .label(tr("moment.task.cancel"))
                            .on_click(move |_, window, cx| {
                                cancel.emit(UiEventKind::CancelPressed, window, cx)
                            }),
                    )
                }),
        )
        .when_some(result(&task, cx), |col, result| col.child(result))
}

fn result(task: &Task, cx: &App) -> Option<AnyElement> {
    let text = task.result.clone()?;
    let (icon, color) = match task.lifecycle {
        Lifecycle::Done => (IconName::CircleCheck, cx.theme().success),
        _ => (IconName::CircleX, cx.theme().danger),
    };
    Some(
        h_flex()
            .id("task-result")
            .gap_2()
            .text_sm()
            .child(Icon::new(icon).small().text_color(color))
            .child(div().min_w_0().child(text))
            .test_support()
            .into_any_element(),
    )
}

fn step_row(ix: usize, step: &TaskStep, refs: &RowRefs, sink: &EventSink, cx: &App) -> AnyElement {
    let (icon, color) = match step.lifecycle {
        Lifecycle::Done => (IconName::CircleCheck, cx.theme().success),
        Lifecycle::InProgress => (IconName::LoaderCircle, cx.theme().foreground),
        Lifecycle::NeedsMe => (IconName::TriangleAlert, cx.theme().warning),
        Lifecycle::Failed | Lifecycle::Dismissed => (IconName::CircleX, cx.theme().danger),
        Lifecycle::Pending => (IconName::Dash, cx.theme().muted_foreground),
    };
    let grant = step.grant_id.as_ref().and_then(|id| refs.grants.get(id));
    let decision = (step.lifecycle == Lifecycle::NeedsMe)
        .then_some(grant)
        .flatten()
        .map(|grant| decision(ix, grant, sink, cx));
    v_flex()
        .id(domain_id("step", &ix))
        .gap_1()
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .child(Icon::new(icon).xsmall().text_color(color))
                .child(div().flex_1().min_w_0().child(step.label.clone()))
                .child(div().text_xs().text_color(color).child(i18n::lifecycle(step.lifecycle))),
        )
        .children(decision)
        .into_any_element()
}

fn decision(ix: usize, grant: &Grant, sink: &EventSink, cx: &App) -> AnyElement {
    let step_sink = sink.scoped(Scope::Block(step_block_id(ix)));
    let deny_sink = step_sink.clone();
    let pending = grant.state == GrantState::Pending;
    v_flex()
        .id("step-decision")
        .ml_5()
        .gap_2()
        .p_2()
        .border_1()
        .border_color(cx.theme().warning)
        .rounded(cx.theme().radius)
        .child(
            h_flex()
                .gap_2()
                .text_xs()
                .child(div().text_color(cx.theme().warning).child(tr_args(
                    "moment.intervention.risk",
                    &[("risk", &i18n::risk(grant.risk))],
                )))
                .child(
                    div()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_color(cx.theme().muted_foreground)
                        .child(grant.scope.clone()),
                ),
        )
        .child(if pending {
            h_flex()
                .gap_2()
                .child(
                    Button::new("step-approve")
                        .primary()
                        .small()
                        .label(tr(if grant.risk.requires_confirmation() {
                            "moment.intervention.approve_hold"
                        } else {
                            "moment.intervention.approve"
                        }))
                        .on_click(move |_, window, cx| {
                            step_sink.emit(UiEventKind::ApprovePressed, window, cx)
                        }),
                )
                .child(
                    Button::new("step-deny")
                        .outline()
                        .small()
                        .label(tr("moment.intervention.deny"))
                        .on_click(move |_, window, cx| {
                            deny_sink.emit(UiEventKind::DenyPressed, window, cx)
                        }),
                )
                .into_any_element()
        } else {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(i18n::grant_state(grant.state))
                .into_any_element()
        })
        .into_any_element()
}
