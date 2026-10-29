// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Task event lines: what a member did to a task (accepted, declined,
//! cancelled it, or decided a step), one compact line in the timeline by
//! the member who signed the event. The task's card shows the task; the
//! line only names the action, the task and the step, and whether the
//! signer's kind is verified for the event (its kind mark, as the source
//! authenticated it; "kind not verified" otherwise).

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, SharedString, div};
use ramflux_desktop_core::{StepId, TaskEventKind, TaskEventPayload};

use super::MomentRow;
use crate::i18n::{self, tr, tr_args};

/// The sentence of a task event line.
pub fn sentence(payload: &TaskEventPayload, row: &MomentRow) -> SharedString {
    let name = if row.outbound { tr("moment.task_event.you") } else { row.author.clone() };
    let task = row.refs.task.as_ref();
    let title = task.map(|t| t.title.clone());
    let step = |id: &Option<StepId>| {
        let id = id.as_ref()?;
        task?.steps.iter().find(|s| &s.id == id).map(|s| s.label.clone())
    };
    let (key, any, object) = match &payload.event {
        TaskEventKind::Accepted => {
            ("moment.task_event.accepted", "moment.task_event.accepted_any", title)
        }
        TaskEventKind::Declined => {
            ("moment.task_event.declined", "moment.task_event.declined_any", title)
        }
        TaskEventKind::Cancelled => {
            ("moment.task_event.cancelled", "moment.task_event.cancelled_any", title)
        }
        TaskEventKind::Approved { step_id } => {
            ("moment.task_event.approved", "moment.task_event.approved_any", step(step_id))
        }
        TaskEventKind::Denied { step_id } => {
            ("moment.task_event.denied", "moment.task_event.denied_any", step(step_id))
        }
    };
    match object {
        Some(object) => tr_args(key, &[("name", &name), ("object", &object)]),
        None => tr_args(any, &[("name", &name)]),
    }
}

/// Renders a task event line, with its time (it has no row header).
pub fn render(payload: &TaskEventPayload, row: &MomentRow, cx: &App) -> impl IntoElement {
    let icon = match payload.event {
        TaskEventKind::Accepted | TaskEventKind::Approved { .. } => IconName::CircleCheck,
        TaskEventKind::Declined | TaskEventKind::Denied { .. } => IconName::CircleX,
        TaskEventKind::Cancelled => IconName::Dash,
    };
    let text = sentence(payload, row);
    // The source's flag on the event says the signer's kind is verified
    // for the event's role, and `actor_kind` is the kind its device's signed
    // profile authenticates (product `carrier_actor_kind`, independent of
    // the role). A verified event therefore names the source's kind
    // directly; the directory's profile-signed claim is only the fallback
    // for a source that sent no kind (else "Kind verified", unnamed).
    // On an event line the source checked the device, so an authenticated kind
    // that is not verified for the action is a role mismatch and is named as
    // such (KIND-01-FU-02).
    let mismatch = payload.actor_kind.filter(|_| !payload.actor_kind_verified);
    let kind = if payload.actor_kind_verified {
        payload.actor_kind.or_else(|| {
            row.refs
                .people
                .get(&row.moment.origin)
                .map(|p| p.kind_claim)
                .filter(|claim| claim.is_verified())
                .and_then(|claim| claim.kind())
        })
    } else {
        None
    };
    h_flex()
        .id("task-event")
        .aria_label(text.clone())
        .gap_2()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(icon).xsmall())
        .child(div().min_w_0().child(text))
        .child(match mismatch {
            Some(actor_kind) => {
                crate::identity::kind_mismatch_mark("task-event-kind".into(), actor_kind, cx)
            }
            None => crate::identity::actor_kind_mark(
                "task-event-kind",
                kind,
                payload.actor_kind_verified,
                cx,
            ),
        })
        .child(div().child(i18n::time_of_day(row.moment.at)))
        .test_support()
}
