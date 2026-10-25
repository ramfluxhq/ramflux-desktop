// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Task moments: what an AI participant does for the user inside the
//! conversation, from proposal to result.
//!
//! The card names the task, who works on it and its state, then its steps
//! and the result. A proposal shows the plan with Start and Decline (only
//! for the requester; agents never confirm). A running task shows its
//! active steps (all of them when expanded, view-local) and, for its
//! requester, Cancel.
//!
//! A step that needs the user asks in exactly one place. An approval is
//! decided right inside the step: the grant it waits on, with Approve and
//! Deny scoped to that step (`Block(step-N)`), through the same grant flow
//! and hold-to-confirm strip as any approval. A choice or an input lives in
//! the agent's surface; the step only points there and brings it into view.
//! Only the requester decides a step; on someone else's task the step says
//! who decides.
//!
//! Trust is stated, not implied: until participants carry a signed kind,
//! that the assignee is an AI participant is only its own claim, and the
//! card says so. A stalled assignee, a compacted history and the source's
//! answer to the user's last request are shown in the card
//! (`task_status`).

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Hsla, IntoElement, SharedString, div};
use ramflux_desktop_core::selectors::TaskEndingKind;
use ramflux_desktop_core::{
    Actor, BlockId, Grant, GrantState, MomentId, NeedsMe, Scope, StepState, Task, TaskState,
    TaskStep, UiEventKind,
};

use super::{RowRefs, RowState};
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args};
use crate::ids::domain_id;
use crate::root::step_block_id;

/// Renders a task progress body.
pub fn render(refs: &RowRefs, sink: &EventSink, state: &RowState, cx: &App) -> impl IntoElement {
    let Some(task) = refs.task.clone() else {
        return v_flex()
            .id("task-progress")
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(tr("moment.task.missing"));
    };
    let assignee = refs.name(&task.assignee);
    let mine = task.requester == Actor::Human;
    let proposal = task.state == TaskState::Proposed;
    let done = task.finished_steps();
    let total = task.steps.len();
    let expanded = state.expanded || proposal;
    let steps: Vec<AnyElement> = task
        .steps
        .iter()
        .enumerate()
        .filter(|(_, step)| {
            expanded
                || matches!(step.state, StepState::Running | StepState::NeedsMe | StepState::Failed)
        })
        .map(|(ix, step)| step_row(ix, step, &assignee, refs, sink, cx))
        .collect();
    // Lossy by design: the bar is only displayed.
    #[allow(clippy::cast_precision_loss)]
    let percent = if total == 0 { 0.0 } else { done as f32 / total as f32 * 100.0 };
    let attention = matches!(task.state, TaskState::NeedsMe | TaskState::Proposed);
    let stalled = task.flags.stalled && !task.state.is_terminal();
    v_flex()
        .id("task-progress")
        .gap_2()
        .p_3()
        .border_1()
        .border_color(if attention { cx.theme().warning } else { cx.theme().border })
        .rounded(cx.theme().radius_lg)
        .bg(cx.theme().group_box)
        .child(header(&task, &assignee, cx))
        .when(proposal, |col| {
            col.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr_args("moment.task.proposal_hint", &[("name", &assignee)])),
            )
        })
        // A cancelled task did not get through its skipped steps: no bar.
        .when(!proposal && task.state != TaskState::Cancelled, |col| {
            col.child(
                Progress::new("task-bar").value(percent).accessibility_label(task.title.clone()),
            )
        })
        .when(stalled, |col| col.child(super::task_status::stalled(&assignee, mine, cx)))
        .children(steps)
        .when_some(task.answer.as_ref(), |col, answer| {
            col.child(super::task_status::answer(answer, &assignee, mine.then_some(sink), cx))
        })
        .when(task.flags.compacted, |col| col.child(super::task_status::compacted(cx)))
        .child(footer(&task, mine, &requester(&task, refs), state.expanded, sink, cx))
        .when_some(result(&task, refs.result_surface.clone(), sink, cx), |col, result| {
            col.child(result)
        })
        .when_some(ending(&task, refs, cx), |col, ending| col.child(ending))
}

/// How a failed or cancelled task without a result ended, by whom: the
/// typed record of its end (a denied step, a decline, a cancel), never an
/// empty summary.
fn ending(task: &Task, refs: &RowRefs, cx: &App) -> Option<AnyElement> {
    if task.result.is_some() {
        return None;
    }
    let (ending, mine) = refs.ending.as_ref()?;
    let name = if *mine { tr("moment.task.ending.you") } else { refs.name(&ending.by) };
    let key = match ending.how {
        TaskEndingKind::Denied | TaskEndingKind::Declined => "moment.task.ending.declined_by",
        TaskEndingKind::Cancelled => "moment.task.ending.cancelled_by",
    };
    let (icon, color) = match ending.how {
        TaskEndingKind::Cancelled => (IconName::Dash, cx.theme().muted_foreground),
        TaskEndingKind::Denied | TaskEndingKind::Declined => (IconName::CircleX, cx.theme().danger),
    };
    let text = tr_args(key, &[("name", &name)]);
    Some(
        h_flex()
            .id("task-ending")
            .aria_label(text.clone())
            .gap_2()
            .text_sm()
            .child(Icon::new(icon).small().text_color(color))
            .child(div().flex_1().min_w_0().child(text))
            .test_support()
            .into_any_element(),
    )
}

fn header(task: &Task, assignee: &SharedString, cx: &App) -> impl IntoElement {
    let state_color = state_color(task.state, cx);
    h_flex()
        .gap_2()
        .child(Icon::new(IconName::Bot).small().text_color(cx.theme().muted_foreground))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().text_sm().font_medium().child(task.title.clone()))
                .child(
                    h_flex()
                        .id("task-state")
                        .gap_1()
                        .text_xs()
                        .child(
                            div().text_color(cx.theme().muted_foreground).child(assignee.clone()),
                        )
                        .when(!task.flags.kind_verified, |row| {
                            row.child(div().text_color(cx.theme().muted_foreground).child("·"))
                                .child(
                                    div()
                                        .id("task-kind")
                                        .text_color(cx.theme().muted_foreground)
                                        .child(tr("moment.task.kind_unverified"))
                                        .test_support(),
                                )
                        })
                        .child(div().text_color(cx.theme().muted_foreground).child("·"))
                        .child(
                            div()
                                .text_color(card_text(state_color, cx))
                                .child(i18n::task_state(task.state)),
                        )
                        .test_support(),
                ),
        )
        .when(!matches!(task.state, TaskState::Proposed | TaskState::Cancelled), |row| {
            row.child(div().text_xs().text_color(cx.theme().muted_foreground).child(tr_args(
                "moment.task.count",
                &[
                    ("done", &task.finished_steps().to_string()),
                    ("total", &task.steps.len().to_string()),
                ],
            )))
        })
}

/// A status color made readable as text on the card's background.
fn card_text(color: Hsla, cx: &App) -> Hsla {
    crate::tone::on(color, cx.theme().group_box)
}

/// The color of a task state, on the card and in its row header.
pub(super) fn state_color(state: TaskState, cx: &App) -> Hsla {
    match state {
        TaskState::Proposed | TaskState::NeedsMe => cx.theme().warning,
        TaskState::Done => cx.theme().success,
        TaskState::Failed => cx.theme().danger,
        TaskState::Running | TaskState::Cancelled => cx.theme().muted_foreground,
    }
}

/// Start / Decline for a proposal, Cancel for a running task (both only
/// for the requester), and the steps disclosure.
fn footer(
    task: &Task,
    mine: bool,
    requester: &SharedString,
    expanded: bool,
    sink: &EventSink,
    cx: &App,
) -> AnyElement {
    let proposal = task.state == TaskState::Proposed;
    let disclosure = sink.scoped(Scope::Block(BlockId::new("steps")));
    let cancel = sink.scoped(Scope::Block(BlockId::new("cancel")));
    let start = sink.scoped(Scope::Block(BlockId::new("start")));
    h_flex()
        .id("task-footer")
        .gap_2()
        .when(proposal && mine, |row| {
            row.child(
                Button::new("task-start")
                    .primary()
                    .small()
                    .label(tr("moment.task.start"))
                    .on_click(move |_, window, cx| {
                        start.emit(UiEventKind::ProposalAccepted, window, cx)
                    }),
            )
        })
        .when(!proposal, |row| {
            row.child(
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
        })
        .when(mine && !task.state.is_terminal(), |row| {
            let label = if proposal { "moment.task.decline" } else { "moment.task.cancel" };
            let button =
                Button::new("task-cancel").label(tr(label)).on_click(move |_, window, cx| {
                    cancel.emit(UiEventKind::CancelPressed, window, cx);
                });
            row.child(if proposal { button.outline().small() } else { button.ghost().xsmall() })
        })
        .when(!mine && proposal, |row| {
            row.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("moment.task.not_yours")),
            )
        })
        .when(!mine && !proposal && !task.state.is_terminal(), |row| {
            row.child(
                div()
                    .id("task-asked-by")
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr_args("moment.task.asked_by", &[("name", requester)]))
                    .test_support(),
            )
        })
        .into_any_element()
}

/// Who asked for the task, by name (empty for this account).
fn requester(task: &Task, refs: &RowRefs) -> SharedString {
    match &task.requester {
        Actor::Agent(id) | Actor::Member(id) => refs.name(id),
        Actor::Human => SharedString::default(),
    }
}

fn result(
    task: &Task,
    surface: Option<MomentId>,
    sink: &EventSink,
    cx: &App,
) -> Option<AnyElement> {
    let text = task.result.clone()?;
    let (icon, color) = match task.state {
        TaskState::Done => (IconName::CircleCheck, cx.theme().success),
        TaskState::Cancelled => (IconName::Dash, cx.theme().muted_foreground),
        _ => (IconName::CircleX, cx.theme().danger),
    };
    let show = surface.map(|target| {
        let sink = sink.scoped(Scope::Block(BlockId::new("result")));
        Button::new("task-show-result")
            .ghost()
            .xsmall()
            .label(tr("moment.task.show_result"))
            .on_click(move |_, window, cx| {
                sink.emit(
                    UiEventKind::FocusMomentRequested { moment_id: target.clone() },
                    window,
                    cx,
                );
            })
    });
    Some(
        h_flex()
            .id("task-result")
            .gap_2()
            .text_sm()
            .child(Icon::new(icon).small().text_color(color))
            .child(div().flex_1().min_w_0().child(text))
            .children(show)
            .test_support()
            .into_any_element(),
    )
}

fn step_row(
    ix: usize,
    step: &TaskStep,
    assignee: &SharedString,
    refs: &RowRefs,
    sink: &EventSink,
    cx: &App,
) -> AnyElement {
    let (icon, color) = match step.state {
        StepState::Done => (IconName::CircleCheck, cx.theme().success),
        StepState::Running => (IconName::LoaderCircle, cx.theme().foreground),
        StepState::NeedsMe => (IconName::TriangleAlert, cx.theme().warning),
        StepState::Failed => (IconName::CircleX, cx.theme().danger),
        StepState::Pending | StepState::Skipped => (IconName::Dash, cx.theme().muted_foreground),
    };
    let asking = (step.state == StepState::NeedsMe).then_some(step.needs_me.as_ref()).flatten();
    let mine = refs.task.as_ref().is_some_and(|t| t.requester == Actor::Human);
    let place = asking.map(|needs_me| match needs_me {
        NeedsMe::Approval { .. } if !mine => {
            let name = refs.task.as_ref().map(|t| requester(t, refs)).unwrap_or_default();
            not_mine(&name, cx)
        }
        NeedsMe::Approval { grant_id } => match refs.grants.get(grant_id) {
            Some(grant) => decision(ix, grant, sink, cx),
            None => missing_place(cx),
        },
        NeedsMe::Choice { .. } | NeedsMe::Input { .. } => {
            pointer(ix, needs_me, assignee, refs.decision_surface.clone(), sink, cx)
        }
    });
    v_flex()
        .id(domain_id("step", &step.id))
        .gap_1()
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .child(Icon::new(icon).xsmall().text_color(color))
                .child(div().flex_1().min_w_0().child(step.label.clone()))
                .child(
                    div()
                        .text_xs()
                        .text_color(card_text(color, cx))
                        .child(i18n::step_state(step.state)),
                ),
        )
        .children(place)
        .test_support()
        .into_any_element()
}

/// A step of someone else's task: its requester decides.
fn not_mine(requester: &SharedString, cx: &App) -> AnyElement {
    div()
        .id("step-not-mine")
        .ml_5()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(tr_args("moment.task.decided_by", &[("name", requester)]))
        .test_support()
        .into_any_element()
}

fn missing_place(cx: &App) -> AnyElement {
    div()
        .ml_5()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(tr("moment.task.place_missing"))
        .into_any_element()
}

/// A choice or an input is answered in the agent's surface: the step says
/// so and brings that surface into view (it never repeats the controls).
fn pointer(
    ix: usize,
    needs_me: &NeedsMe,
    assignee: &SharedString,
    surface: Option<MomentId>,
    sink: &EventSink,
    cx: &App,
) -> AnyElement {
    let key = match needs_me {
        NeedsMe::Choice { .. } => "moment.task.choose_in_view",
        _ => "moment.task.answer_in_view",
    };
    let sink = sink.scoped(Scope::Block(step_block_id(ix)));
    h_flex()
        .id("step-pointer")
        .ml_5()
        .gap_2()
        .text_xs()
        .text_color(card_text(cx.theme().warning, cx))
        .child(div().flex_1().min_w_0().child(tr_args(key, &[("name", assignee)])))
        .when_some(surface, |row, target| {
            row.child(
                Button::new("step-show-view")
                    .ghost()
                    .xsmall()
                    .label(tr("moment.task.show_view"))
                    .on_click(move |_, window, cx| {
                        sink.emit(
                            UiEventKind::FocusMomentRequested { moment_id: target.clone() },
                            window,
                            cx,
                        );
                    }),
            )
        })
        .test_support()
        .into_any_element()
}

fn decision(ix: usize, grant: &Grant, sink: &EventSink, cx: &App) -> AnyElement {
    let step_sink = sink.scoped(Scope::Block(step_block_id(ix)));
    let deny_sink = step_sink.clone();
    let open = grant.state == GrantState::Pending && grant.failure.is_none();
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
                .child(div().text_color(card_text(cx.theme().warning, cx)).child(tr_args(
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
        .child(match grant.failure {
            Some(failure) => super::grant_failure::render(failure, cx),
            None if open => h_flex()
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
                .into_any_element(),
            None => div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(i18n::grant_state(grant.state))
                .into_any_element(),
        })
        .test_support()
        .into_any_element()
}
