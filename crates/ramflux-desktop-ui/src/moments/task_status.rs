// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! What a task card says beyond its steps: an assignee that stalled, a
//! compacted history, and the source's answer to the user's last request
//! about the task (a decision on its way, or why nothing was done). Each
//! one says its meaning in words; color only supports it.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Hsla, SharedString, div};
use ramflux_desktop_core::{TaskAnswer, TaskRefusal};

use crate::i18n::{tr, tr_args};

fn notice(
    id: &'static str,
    icon: IconName,
    color: Hsla,
    title: SharedString,
    text: SharedString,
    cx: &App,
) -> AnyElement {
    h_flex()
        .id(id)
        .gap_2()
        .items_start()
        .p_2()
        .border_1()
        .border_color(color)
        .rounded(cx.theme().radius)
        .child(Icon::new(icon).small().text_color(color))
        .child(
            v_flex()
                .min_w_0()
                .gap_1()
                .child(div().text_sm().font_medium().child(title))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(text)),
        )
        .test_support()
        .into_any_element()
}

fn quiet(id: &'static str, icon: IconName, text: SharedString, cx: &App) -> AnyElement {
    h_flex()
        .id(id)
        .gap_1()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(icon).xsmall())
        .child(div().min_w_0().child(text))
        .test_support()
        .into_any_element()
}

/// The assignee used up what it may send: the task stays where it is.
pub fn stalled(assignee: &SharedString, mine: bool, cx: &App) -> AnyElement {
    let text = if mine { "moment.task.stalled.text_mine" } else { "moment.task.stalled.text" };
    notice(
        "task-stalled",
        IconName::TriangleAlert,
        cx.theme().warning,
        tr_args("moment.task.stalled.title", &[("name", assignee)]),
        tr_args(text, &[("name", assignee)]),
        cx,
    )
}

/// The history behind a task was compacted; what it reached is kept.
pub fn compacted(cx: &App) -> AnyElement {
    quiet("task-compacted", IconName::Inbox, tr("moment.task.compacted"), cx)
}

/// The source's answer to the user's last request about the task.
pub fn answer(answer: &TaskAnswer, assignee: &SharedString, cx: &App) -> AnyElement {
    let refusal = match answer {
        TaskAnswer::DecisionPending { .. } => {
            let text = tr_args("moment.task.answer.pending", &[("name", assignee)]);
            return quiet("task-answer", IconName::LoaderCircle, text, cx);
        }
        TaskAnswer::DecisionUnconfirmed { .. } => {
            return notice(
                "task-answer",
                IconName::TriangleAlert,
                cx.theme().warning,
                tr_args("moment.task.answer.unconfirmed.title", &[("name", assignee)]),
                tr("moment.task.answer.unconfirmed.text"),
                cx,
            );
        }
        TaskAnswer::Refused(refusal) => refusal,
    };
    let (title, text) = match refusal {
        TaskRefusal::Changed => {
            (tr("moment.task.refused.changed.title"), tr("moment.task.refused.changed.text"))
        }
        TaskRefusal::Finished => {
            (tr("moment.task.refused.title"), tr("moment.task.refused.finished"))
        }
        TaskRefusal::NotAllowed => {
            (tr("moment.task.refused.title"), tr("moment.task.refused.not_allowed"))
        }
        TaskRefusal::AlreadyDecided => {
            (tr("moment.task.refused.title"), tr("moment.task.refused.already_decided"))
        }
        TaskRefusal::TooLarge => {
            (tr("moment.task.refused.title"), tr("moment.task.refused.too_large"))
        }
        TaskRefusal::Stalled => {
            (tr("moment.task.refused.title"), tr("moment.task.refused.stalled"))
        }
        TaskRefusal::Busy => (tr("moment.task.refused.title"), tr("moment.task.refused.busy")),
        TaskRefusal::DecisionRejected => (
            tr_args("moment.task.refused.rejected.title", &[("name", assignee)]),
            tr("moment.task.refused.rejected.text"),
        ),
        TaskRefusal::Other(reason) => (
            tr("moment.task.refused.title"),
            tr_args(
                "moment.task.refused.other",
                &[("reason", &SharedString::from(reason.clone()))],
            ),
        ),
    };
    notice("task-answer", IconName::Info, cx.theme().danger, title, text, cx)
}
