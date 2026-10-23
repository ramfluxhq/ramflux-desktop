// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Inline moment rows of the conversation field.
//!
//! A [`MomentRow`] is a read-only presentation snapshot built from the store
//! (the moment plus everything it references). [`MomentView`] renders it with
//! the row chrome (day header, author, local time, lifecycle) and dispatches
//! the body to the renderer for its kind. Every [`MomentKind`] has a
//! renderer; bodies expand and collapse in place.
//!
//! Rows emit UI events through the moment-scoped [`EventSink`] they are given
//! and read view-local state ([`RowState`]) owned by the conversation field;
//! they never touch the store.

mod conflict;
mod contact;
mod governance;
mod handoff;
mod intervention;
mod message;
mod notice;
mod object_transfer;
mod privacy;
mod safety;
pub mod surface;
mod task_progress;

use std::collections::BTreeMap;
use std::rc::Rc;

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::diff::DiffState;
use gpui_kit::component::input::InputState;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, IntoElement, SharedString, Window, div};
use ramflux_desktop_core::{
    Actor, AuditEntry, BlockId, BlockKind, Grant, GrantId, Lifecycle, Moment, MomentId, MomentKind,
    Object, ObjectId, Participant, ParticipantId, Store, Task, selectors,
};

use crate::bubble::EventSink;
use crate::i18n;
use crate::ids::domain_id;

pub use conflict::compare_patch as conflict_patch;
pub use intervention::{DECISION_BLOCK, DETAILS_BLOCK};
pub use object_transfer::DOWNLOADS;

/// Everything a moment references, resolved from the store.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowRefs {
    /// Grants referenced by an intervention, approval blocks or task steps.
    pub grants: BTreeMap<GrantId, Grant>,
    /// Objects referenced by a transfer, an attachment or a conflict.
    pub objects: BTreeMap<ObjectId, Object>,
    /// The task a progress moment presents.
    pub task: Option<Task>,
    /// Participants the payload names (targets, devices, requesters).
    pub people: BTreeMap<ParticipantId, Participant>,
    /// Audit records about the moment or its grants, oldest first.
    pub audit: Vec<AuditEntry>,
    /// For an intervention whose grant a task step owns: that step. The
    /// decision then renders only inside the task (one moment, one place).
    pub owner: Option<OwningStep>,
}

/// The task step an intervention belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwningStep {
    /// The task progress moment.
    pub moment_id: MomentId,
    /// Task title.
    pub title: String,
    /// One-based step number, for display.
    pub number: usize,
    /// Step label.
    pub label: String,
}

impl RowRefs {
    /// Display name of a referenced participant, else its id.
    pub fn name(&self, id: &ParticipantId) -> SharedString {
        self.people.get(id).map_or_else(|| id.to_string().into(), |p| p.display.clone().into())
    }
}

/// Presentation snapshot of one moment.
#[derive(Debug, Clone, PartialEq)]
pub struct MomentRow {
    /// The moment.
    pub moment: Moment,
    /// Display name of the origin participant.
    pub author: SharedString,
    /// Resolved references.
    pub refs: RowRefs,
    /// Day header shown above the first moment of a local day.
    pub day_header: Option<SharedString>,
    /// The local user wrote it (receipts are shown on outbound messages).
    pub outbound: bool,
}

impl MomentRow {
    /// Builds the snapshot for `moment`; `me` is the local user's participant.
    pub fn build(store: &Store, moment: &Moment, me: Option<&ParticipantId>) -> Self {
        let author = store.participant(&moment.origin).map_or_else(
            || SharedString::from(moment.origin.to_string()),
            |p| p.display.clone().into(),
        );
        Self {
            moment: moment.clone(),
            author,
            refs: refs(store, moment),
            day_header: None,
            outbound: me == Some(&moment.origin),
        }
    }
}

fn refs(store: &Store, moment: &Moment) -> RowRefs {
    let mut refs = RowRefs::default();
    let grant = |id: &GrantId, refs: &mut RowRefs| {
        if let Some(g) = store.grant(id) {
            refs.grants.insert(id.clone(), g.clone());
        }
    };
    let object = |id: &ObjectId, refs: &mut RowRefs| {
        if let Some(o) = store.object(id) {
            refs.objects.insert(id.clone(), o.clone());
        }
    };
    let people: Vec<&ParticipantId> = match &moment.kind {
        MomentKind::Message(p) => {
            if let Some(id) = &p.attachment {
                object(id, &mut refs);
            }
            Vec::new()
        }
        MomentKind::AgentSurface(p) => {
            for block in p.blocks.iter().filter(|b| b.kind == BlockKind::Approval) {
                if let Some(id) = block.data.get("grant_id").and_then(|v| v.as_str()) {
                    grant(&GrantId::new(id), &mut refs);
                }
            }
            Vec::new()
        }
        MomentKind::ObjectTransfer(p) => {
            object(&p.object_id, &mut refs);
            Vec::new()
        }
        MomentKind::Intervention(p) => {
            grant(&p.grant_id, &mut refs);
            refs.owner = selectors::owning_task_step(store, &p.grant_id).and_then(|owner| {
                let task = store.task(&owner.task_id)?;
                let step = task.steps.get(owner.step)?;
                Some(OwningStep {
                    moment_id: owner.moment_id,
                    title: task.title.clone(),
                    number: owner.step + 1,
                    label: step.label.clone(),
                })
            });
            Vec::new()
        }
        MomentKind::Conflict(p) => {
            object(&p.object_id, &mut refs);
            Vec::new()
        }
        MomentKind::TaskProgress(p) => {
            refs.task = store.task(&p.task_id).cloned();
            let steps: Vec<GrantId> = refs
                .task
                .iter()
                .flat_map(|t| t.steps.iter().filter_map(|s| s.grant_id.clone()))
                .collect();
            for id in &steps {
                grant(id, &mut refs);
            }
            Vec::new()
        }
        MomentKind::SafetyChange(p) => vec![&p.participant_id],
        MomentKind::Governance(p) => vec![&p.target],
        MomentKind::ContactRequest(p) => vec![&p.from],
        MomentKind::Handoff(p) => p.to_device.iter().chain([&p.from_device]).collect(),
        MomentKind::PrivacyReveal(p) => p.revealed.iter().map(|r| &r.by).collect(),
        MomentKind::Fallback(_) | MomentKind::System(_) => Vec::new(),
    };
    for id in people {
        if let Some(p) = store.participant(id) {
            refs.people.insert(id.clone(), p.clone());
        }
    }
    refs.audit = store
        .audit()
        .iter()
        .filter(|entry| {
            entry.subject == moment.id.as_str()
                || refs.grants.keys().any(|grant| grant.as_str() == entry.subject)
        })
        .cloned()
        .collect();
    for entry in &refs.audit {
        if let Actor::Agent(id) = &entry.proposed_by
            && let Some(p) = store.participant(id)
        {
            refs.people.insert(id.clone(), p.clone());
        }
    }
    refs
}

/// View-local state of one row, owned by the conversation field (rule R4).
#[derive(Clone, Default)]
pub struct RowState {
    /// The row's disclosure is expanded.
    pub expanded: bool,
    /// The row is the target of "take me to what needs me".
    pub highlighted: bool,
    /// Selected values of surface select blocks, by block.
    pub fields: Rc<BTreeMap<BlockId, String>>,
    /// Text inputs of surface input blocks, by block.
    pub inputs: Rc<BTreeMap<BlockId, Entity<InputState>>>,
    /// Compare state of a conflict.
    pub diff: Option<Entity<DiffState>>,
}

/// One rendered moment row.
#[derive(IntoElement)]
pub struct MomentView {
    row: MomentRow,
    sink: EventSink,
    state: RowState,
}

impl MomentView {
    /// A row emitting through `sink`, which must already be scoped to the moment.
    pub fn new(row: MomentRow, sink: EventSink, state: RowState) -> Self {
        Self { row, sink, state }
    }
}

impl RenderOnce for MomentView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { row, sink, state } = self;
        let moment = &row.moment;
        let lifecycle = moment.lifecycle;
        let body = match &moment.kind {
            MomentKind::Message(p) => {
                message::render(p, &row.refs, row.outbound, &sink, cx).into_any_element()
            }
            MomentKind::AgentSurface(p) => {
                surface::SurfaceBody::new(p.clone(), &row.refs, sink.clone(), &state)
                    .into_any_element()
            }
            MomentKind::ObjectTransfer(p) => {
                object_transfer::render(p, &row.refs, &sink, &state, cx).into_any_element()
            }
            MomentKind::Intervention(_) if row.refs.owner.is_some() => {
                intervention::task_reference(row.refs.owner.as_ref(), &sink, cx).into_any_element()
            }
            MomentKind::Intervention(p) => {
                intervention::InterventionBody::new(p.clone(), &row.refs, lifecycle, sink.clone())
                    .proposed_by(row.author.clone())
                    .expanded(state.expanded)
                    .into_any_element()
            }
            MomentKind::SafetyChange(p) => {
                safety::render(p, &row.refs, lifecycle, &sink, cx).into_any_element()
            }
            MomentKind::Governance(p) => governance::render(p, &row.refs, cx).into_any_element(),
            MomentKind::ContactRequest(p) => {
                contact::render(p, &row.refs, lifecycle, &sink, cx).into_any_element()
            }
            MomentKind::TaskProgress(_) => {
                task_progress::render(&row.refs, lifecycle, &sink, &state, cx).into_any_element()
            }
            MomentKind::Conflict(p) => {
                conflict::render(p, &row.refs, lifecycle, &sink, &state, cx).into_any_element()
            }
            MomentKind::PrivacyReveal(p) => {
                privacy::render(p, &row.refs, lifecycle, &sink, cx).into_any_element()
            }
            MomentKind::Handoff(p) => {
                handoff::render(p, &row.refs, lifecycle, &sink, cx).into_any_element()
            }
            MomentKind::System(p) => notice::system(p, cx).into_any_element(),
            MomentKind::Fallback(p) => notice::fallback(p, cx).into_any_element(),
        };
        let _ = window;
        v_flex()
            .id(domain_id("moment", &moment.id))
            .w_full()
            .min_w_0()
            .gap_1()
            .when_some(row.day_header.clone(), |col, day| col.child(day_header(day, cx)))
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .rounded(cx.theme().radius)
                    .when(state.highlighted, |r| r.bg(cx.theme().muted))
                    .child(header(&row, cx))
                    .child(body),
            )
    }
}

fn day_header(day: SharedString, cx: &App) -> impl IntoElement {
    h_flex()
        .w_full()
        .gap_2()
        .pt_2()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(div().h_px().flex_1().bg(cx.theme().border))
        .child(day)
        .child(div().h_px().flex_1().bg(cx.theme().border))
}

fn header(row: &MomentRow, cx: &App) -> impl IntoElement {
    let moment = &row.moment;
    let marker = match moment.lifecycle {
        Lifecycle::Done => None,
        Lifecycle::NeedsMe => Some(cx.theme().warning),
        Lifecycle::Failed => Some(cx.theme().danger),
        Lifecycle::Pending | Lifecycle::InProgress | Lifecycle::Dismissed => {
            Some(cx.theme().muted_foreground)
        }
    };
    h_flex()
        .gap_2()
        .text_xs()
        .child(div().font_semibold().text_color(cx.theme().foreground).child(row.author.clone()))
        .child(div().text_color(cx.theme().muted_foreground).child(i18n::time_of_day(moment.at)))
        .when_some(marker, |header, color| {
            header.child(div().text_color(color).child(i18n::lifecycle(moment.lifecycle)))
        })
}
