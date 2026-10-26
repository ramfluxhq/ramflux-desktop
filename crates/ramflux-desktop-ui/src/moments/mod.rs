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

pub mod call;
mod conflict;
mod contact;
mod governance;
pub mod grant_failure;
mod handoff;
mod intervention;
mod message;
mod name_change;
mod notice;
mod object_transfer;
mod privacy;
mod safety;
pub mod surface;
mod task_event;
mod task_progress;
mod task_status;

use std::collections::BTreeMap;
use std::rc::Rc;

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::ThemeStyled as _;
use gpui_kit::component::diff::DiffState;
use gpui_kit::component::input::InputState;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, FocusHandle, IntoElement, Role, SharedString, Window, div};
use ramflux_desktop_core::{
    Actor, AuditEntry, BlockId, BlockKind, Call, Grant, GrantId, Lifecycle, Moment, MomentId,
    MomentKind, Object, ObjectId, Participant, ParticipantId, Store, Task, selectors,
};

use crate::bubble::EventSink;
use crate::i18n;
use crate::ids::domain_id;

pub use conflict::compare_patch as conflict_patch;
pub use intervention::{DECISION_BLOCK, DETAILS_BLOCK};
pub use object_transfer::save_with_dialog;

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
    /// For a task whose step asks for a choice or an input: the surface
    /// moment where the user answers (the task only points there).
    pub decision_surface: Option<MomentId>,
    /// For a task with a result surface: its moment.
    pub result_surface: Option<MomentId>,
    /// For a failed or cancelled task without a result: how it ended and
    /// who ended it, and whether that was the local user.
    pub ending: Option<(selectors::TaskEnding, bool)>,
    /// The call a call record presents.
    pub call: Option<Call>,
    /// For an ended call: whether a call back could be placed now.
    pub call_back: bool,
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
    /// Display name of a referenced participant, else its short id.
    pub fn name(&self, id: &ParticipantId) -> SharedString {
        self.people.get(id).map_or_else(|| id.short().into(), |p| p.display.clone().into())
    }
}

/// Audit records by subject (moment or grant id) with their position in
/// the store's audit trail, built once per refresh instead of per row.
#[derive(Debug, Default)]
pub struct AuditIndex<'a> {
    by_subject: BTreeMap<&'a str, Vec<(usize, &'a AuditEntry)>>,
}

impl<'a> AuditIndex<'a> {
    /// Indexes the store's audit trail.
    pub fn new(store: &'a Store) -> Self {
        let mut by_subject: BTreeMap<&str, Vec<(usize, &AuditEntry)>> = BTreeMap::new();
        for (ix, entry) in store.audit().iter().enumerate() {
            by_subject.entry(entry.subject.as_str()).or_default().push((ix, entry));
        }
        Self { by_subject }
    }

    /// Records about any of `subjects`, oldest first.
    fn about<'s>(&self, subjects: impl Iterator<Item = &'s str>) -> Vec<AuditEntry> {
        let mut found: Vec<(usize, &AuditEntry)> =
            subjects.filter_map(|s| self.by_subject.get(s)).flatten().copied().collect();
        found.sort_by_key(|(ix, _)| *ix);
        found.dedup_by_key(|(ix, _)| *ix);
        found.into_iter().map(|(_, entry)| entry.clone()).collect()
    }
}

/// Presentation snapshot of one moment.
#[derive(Debug, Clone, PartialEq)]
pub struct MomentRow {
    /// The moment.
    pub moment: Moment,
    /// A message's text while its body is in memory; `None` shows a
    /// placeholder and asks the source for it again.
    pub body: Option<SharedString>,
    /// Display name of the origin participant.
    pub author: SharedString,
    /// The warning on the author's name, if its name could pass for
    /// another principal's (shown with the author's id in the row header).
    pub author_warning: Option<SharedString>,
    /// Resolved references.
    pub refs: RowRefs,
    /// Day header shown above the first moment of a local day.
    pub day_header: Option<SharedString>,
    /// The local user wrote it (receipts are shown on outbound messages).
    pub outbound: bool,
    /// For a surface: its version and renderer verdict.
    pub surface: Option<surface::SurfaceStatus>,
}

impl MomentRow {
    /// Builds the snapshot for `moment`; `me` is the local user's participant.
    pub fn build(store: &Store, moment: &Moment, me: Option<&ParticipantId>) -> Self {
        Self::build_with(store, moment, me, &AuditIndex::new(store))
    }

    /// [`MomentRow::build`] with an audit index shared across many rows.
    pub fn build_with(
        store: &Store,
        moment: &Moment,
        me: Option<&ParticipantId>,
        audit: &AuditIndex<'_>,
    ) -> Self {
        let origin = store.participant(&moment.origin);
        let author = origin.map_or_else(
            || SharedString::from(moment.origin.short()),
            |p| p.display.clone().into(),
        );
        let author_warning = origin.and_then(crate::identity::name_warning_text);
        let body = match &moment.kind {
            MomentKind::Message(_) => {
                store.bodies().get(&moment.id).map(|text| SharedString::from(text.clone()))
            }
            _ => None,
        };
        Self {
            moment: moment.clone(),
            body,
            author,
            author_warning,
            refs: refs(store, moment, audit),
            day_header: None,
            outbound: me == Some(&moment.origin),
            surface: surface::SurfaceStatus::of(store, moment),
        }
    }

    /// A message whose text is not in memory (evicted or dropped on lock).
    pub fn is_missing_body(&self) -> bool {
        matches!(&self.moment.kind, MomentKind::Message(p) if !p.tombstone) && self.body.is_none()
    }
}

fn refs(store: &Store, moment: &Moment, audit: &AuditIndex<'_>) -> RowRefs {
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
            let steps: Vec<GrantId> =
                refs.task.iter().flat_map(|t| t.step_grants().cloned()).collect();
            for id in &steps {
                grant(id, &mut refs);
            }
            refs.decision_surface = selectors::task_decision_surface(store, moment).cloned();
            refs.result_surface = refs
                .task
                .as_ref()
                .and_then(|t| t.result_ref.as_ref())
                .and_then(|s| store.surface_moment(s))
                .cloned();
            refs.ending =
                refs.task.as_ref().and_then(|t| selectors::task_ending(store, t)).map(|ending| {
                    let mine = selectors::self_participant(store).as_ref() == Some(&ending.by);
                    (ending, mine)
                });
            let mut people = refs
                .task
                .as_ref()
                .map(|t| match &t.requester {
                    Actor::Agent(id) | Actor::Member(id) => vec![&t.assignee, id],
                    Actor::Human => vec![&t.assignee],
                })
                .unwrap_or_default();
            if let Some((ending, _)) = &refs.ending {
                people.push(&ending.by);
            }
            people
        }
        MomentKind::TaskEvent(p) => {
            refs.task = store.task(&p.task_id).cloned();
            vec![&moment.origin]
        }
        MomentKind::NameChange(p) => vec![&p.participant_id],
        MomentKind::SafetyChange(p) => vec![&p.participant_id],
        MomentKind::Governance(p) => vec![&p.target],
        MomentKind::ContactRequest(p) => vec![&p.from],
        MomentKind::Handoff(p) => p.to_device.iter().chain([&p.from_device]).collect(),
        MomentKind::PrivacyReveal(p) => p.revealed.iter().map(|r| &r.by).collect(),
        MomentKind::Fallback(_) | MomentKind::System(_) => Vec::new(),
        MomentKind::Call(p) => {
            refs.call = store.call(&p.call_id).cloned();
            refs.call_back = ramflux_desktop_core::check_start_call(store, &moment.context_id)
                .is_ok()
                && store.unsupported_reason(ramflux_desktop_core::methods::CALL_INVITE).is_none();
            refs.call.as_ref().map(|c| vec![&c.peer]).unwrap_or_default()
        }
    };
    for id in people {
        if let Some(p) = store.participant(id) {
            refs.people.insert(id.clone(), p.clone());
        }
    }
    refs.audit = audit
        .about(std::iter::once(moment.id.as_str()).chain(refs.grants.keys().map(|g| g.as_str())));
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
    /// Keyboard focus of the highlighted row ("take me to what needs me"
    /// moves focus here; Tab then reaches the row's own controls).
    pub focus: Option<FocusHandle>,
    /// A pending confirmation on this surface was cancelled because the
    /// surface changed; the user reviews it again.
    pub review_again: bool,
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
                message::render(p, row.body.as_ref(), &row.refs, row.outbound, &sink, cx)
                    .into_any_element()
            }
            MomentKind::AgentSurface(p) => match row.surface.clone() {
                Some(status) => {
                    surface::SurfaceBody::new(p.clone(), status, &row.refs, sink.clone(), &state)
                        .into_any_element()
                }
                None => div().into_any_element(),
            },
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
                task_progress::render(&row.refs, &sink, &state, cx).into_any_element()
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
            MomentKind::TaskEvent(p) => task_event::render(p, &row, cx).into_any_element(),
            MomentKind::Call(_) => call::render(&row.refs, &sink, cx).into_any_element(),
            MomentKind::NameChange(p) => name_change::render(p, &row, cx).into_any_element(),
        };
        // A task event and a name change are one compact line each: they
        // carry their own time.
        let compact = matches!(moment.kind, MomentKind::TaskEvent(_) | MomentKind::NameChange(_));
        let focused = state.focus.as_ref().is_some_and(|f| f.is_focused(window));
        v_flex()
            .id(domain_id("moment", &moment.id))
            .role(Role::ListItem)
            .aria_label(accessible_label(&row))
            .aria_selected(state.highlighted)
            .test_support()
            .when_some(state.focus.as_ref(), |el, focus| el.track_focus(focus))
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
                    .when(focused, |r| r.focus_ring_style(window, cx))
                    .when(!compact, |col| col.child(header(&row, cx)))
                    .child(body),
            )
    }
}

/// What assistive technology announces for a row: its kind, author, time
/// and lifecycle state, plus the risk of a decision.
pub fn accessible_label(row: &MomentRow) -> SharedString {
    let moment = &row.moment;
    let base = i18n::tr_args(
        "moment.a11y.row",
        &[
            ("kind", &i18n::moment_kind(&moment.kind)),
            ("author", &row.author),
            ("time", &i18n::time_of_day(moment.at)),
            ("state", &state_label(row).0),
        ],
    );
    match &moment.kind {
        MomentKind::Intervention(p) => {
            i18n::tr_args("moment.a11y.with_risk", &[("row", &base), ("risk", &i18n::risk(p.risk))])
        }
        _ => base,
    }
}

/// A day header: its rules share the row content's inset (`px_2`), so they
/// end where the cards below end.
fn day_header(day: SharedString, cx: &App) -> impl IntoElement {
    h_flex()
        .w_full()
        .gap_2()
        .px_2()
        .pt_2()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(div().h_px().flex_1().bg(cx.theme().border))
        .child(day)
        .child(div().h_px().flex_1().bg(cx.theme().border))
}

/// The state a row names, and whether its header shows it. A task row
/// names its task's state, in every state, with the words of its card; any
/// other row names its lifecycle, except a quiet `Done` (a delivered
/// message is not news).
fn state_label(row: &MomentRow) -> (SharedString, Option<StateTone>) {
    if let (MomentKind::TaskProgress(_), Some(task)) = (&row.moment.kind, &row.refs.task) {
        return (i18n::task_state(task.state), Some(StateTone::Task(task.state)));
    }
    let tone = match row.moment.lifecycle {
        Lifecycle::Done => None,
        lifecycle => Some(StateTone::Lifecycle(lifecycle)),
    };
    (i18n::lifecycle(row.moment.lifecycle), tone)
}

/// Which color a shown state takes.
#[derive(Clone, Copy)]
enum StateTone {
    Task(ramflux_desktop_core::TaskState),
    Lifecycle(Lifecycle),
}

impl StateTone {
    fn color(self, cx: &App) -> gpui_kit::Hsla {
        match self {
            Self::Task(state) => task_progress::state_color(state, cx),
            Self::Lifecycle(Lifecycle::NeedsMe) => cx.theme().warning,
            Self::Lifecycle(Lifecycle::Failed) => cx.theme().danger,
            Self::Lifecycle(Lifecycle::Done) => cx.theme().success,
            Self::Lifecycle(Lifecycle::Pending | Lifecycle::InProgress | Lifecycle::Dismissed) => {
                cx.theme().muted_foreground
            }
        }
    }
}

fn header(row: &MomentRow, cx: &App) -> impl IntoElement {
    let moment = &row.moment;
    let (label, tone) = state_label(row);
    let marker = tone.map(|tone| (label, crate::tone::text(tone.color(cx), cx)));
    h_flex()
        .gap_2()
        .text_xs()
        .child(div().font_semibold().text_color(cx.theme().foreground).child(row.author.clone()))
        // A name that could pass for another's: the warning and the id,
        // next to the name (never a mark on the name itself).
        .when_some(row.author_warning.clone(), |header, warning| {
            header.child(
                h_flex()
                    .id("author-name-warning")
                    .aria_label(warning)
                    .test_support()
                    .gap_1()
                    .text_color(crate::tone::text(cx.theme().warning, cx))
                    .child(
                        gpui_kit::component::Icon::new(gpui_kit::assets::IconName::TriangleAlert)
                            .xsmall(),
                    )
                    .child(
                        div()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_color(cx.theme().muted_foreground)
                            .child(moment.origin.short()),
                    ),
            )
        })
        .child(div().text_color(cx.theme().muted_foreground).child(i18n::time_of_day(moment.at)))
        .when_some(marker, |header, (label, color)| {
            header.child(div().id("row-state").text_color(color).child(label).test_support())
        })
        .when_some(
            row.surface
                .as_ref()
                .and_then(|s| s.updated_at)
                .and_then(|updated| surface::updated_mark(moment.at, updated, cx)),
            |header, mark| header.child(mark),
        )
        .when(matches!(&moment.kind, MomentKind::AgentSurface(p) if p.streaming), |header| {
            header.child(surface::writing_mark(cx))
        })
}
