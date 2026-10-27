// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The conversation field's row list: presentation snapshots of the shown
//! context's moments, in timeline order, with local-day headers.
//!
//! A change set that only adds or changes moments of the shown context is
//! patched in place: changed rows are rebuilt and rows after the last one
//! are appended, so a new message costs one row, not the whole context.
//! Anything else (a reset, a removed or out-of-order moment, a change to
//! what rows reference) rebuilds the list from the store's timeline index.

use std::collections::HashMap;
use std::rc::Rc;

use ramflux_desktop_core::{ContextId, Moment, MomentId, MomentKind, Store, Timestamp, selectors};

use crate::app::{ChangeSet, StoreHandle};
use crate::i18n;
use crate::moments::{AuditIndex, MomentRow};

/// How the virtual list must follow a row update.
#[derive(Debug, Default, PartialEq, Eq)]
pub enum RowUpdate {
    /// Nothing changed.
    #[default]
    None,
    /// The whole list was replaced.
    Reset,
    /// Rows at these indexes changed and `appended` rows were added at the end.
    Patched {
        /// Changed row indexes.
        changed: Vec<usize>,
        /// Rows added at the end.
        appended: usize,
    },
}

/// The rows of one context plus the lookups that keep updates local.
#[derive(Default)]
pub struct Rows {
    rows: Rc<Vec<MomentRow>>,
    index: HashMap<MomentId, usize>,
    stateful: Vec<usize>,
}

/// Rows that need retained view state (surface inputs, conflict compare).
fn is_stateful(row: &MomentRow) -> bool {
    matches!(row.moment.kind, MomentKind::AgentSurface(_) | MomentKind::Conflict(_))
}

fn key(moment: &Moment) -> (Timestamp, &MomentId) {
    (moment.at, &moment.id)
}

impl Rows {
    /// The rows, shareable with the row renderer.
    pub fn shared(&self) -> Rc<Vec<MomentRow>> {
        Rc::clone(&self.rows)
    }

    /// The rows.
    pub fn as_slice(&self) -> &[MomentRow] {
        &self.rows
    }

    /// Index of a moment's row.
    pub fn position(&self, id: &MomentId) -> Option<usize> {
        self.index.get(id).copied()
    }

    /// Rows that need retained view state.
    pub fn stateful(&self) -> impl Iterator<Item = &MomentRow> {
        self.stateful.iter().filter_map(|ix| self.rows.get(*ix))
    }

    /// Drops every row.
    pub fn clear(&mut self) -> RowUpdate {
        *self = Self::default();
        RowUpdate::Reset
    }

    /// Brings the rows of `context` up to date with `changes` (a full
    /// rebuild when `changes` is `None` or cannot be patched).
    pub fn sync(
        &mut self,
        handle: &StoreHandle,
        context: &ContextId,
        changes: Option<&ChangeSet>,
    ) -> RowUpdate {
        if let Some(changes) = changes.filter(|c| !c.touches_row_refs(handle.store(), context))
            && let Some(update) = self.patch(handle, context, changes)
        {
            return update;
        }
        self.rebuild(handle, context);
        RowUpdate::Reset
    }

    fn rebuild(&mut self, handle: &StoreHandle, context: &ContextId) {
        let store = handle.store();
        let now = handle.session().map(|s| s.now()).unwrap_or_default();
        let me = selectors::self_participant(store);
        let audit = AuditIndex::new(store);
        let mut rows = Vec::with_capacity(store.context_moment_count(context));
        let mut last_day = None;
        for moment in store.context_moments(context) {
            rows.push(day_row(store, moment, me.as_ref(), &audit, now, &mut last_day));
        }
        self.index = rows.iter().enumerate().map(|(ix, r)| (r.moment.id.clone(), ix)).collect();
        self.stateful =
            rows.iter().enumerate().filter(|(_, r)| is_stateful(r)).map(|(ix, _)| ix).collect();
        self.rows = Rc::new(rows);
    }

    /// Patches changed and appended rows; `None` when the change cannot be
    /// expressed that way.
    fn patch(
        &mut self,
        handle: &StoreHandle,
        context: &ContextId,
        changes: &ChangeSet,
    ) -> Option<RowUpdate> {
        let store = handle.store();
        let mut changed = Vec::new();
        let mut added: Vec<&Moment> = Vec::new();
        for id in changes.moments() {
            match store.moment(id) {
                // A removed row shifts every index after it: rebuild.
                None if self.index.contains_key(id) => return None,
                None => {}
                Some(moment) if &moment.context_id != context => {}
                Some(moment) => match self.index.get(id) {
                    Some(ix) => changed.push((*ix, moment)),
                    None => added.push(moment),
                },
            }
        }
        self.add_surface_dependents(store, context, changes, &mut changed);
        added.sort_by(|a, b| key(a).cmp(&key(b)));
        let last = self.rows.last().map(|r| (r.moment.at, r.moment.id.clone()));
        if let (Some(first), Some((at, id))) = (added.first(), &last)
            && key(first) < (*at, id)
        {
            return None;
        }
        if changed.is_empty() && added.is_empty() {
            return Some(RowUpdate::None);
        }
        let now = handle.session().map(|s| s.now()).unwrap_or_default();
        let me = selectors::self_participant(store);
        let audit = AuditIndex::new(store);
        let rows = Rc::make_mut(&mut self.rows);
        let mut touched = Vec::new();
        for (ix, moment) in changed {
            let mut row = MomentRow::build_with(store, moment, me.as_ref(), &audit);
            row.day_header = rows[ix].day_header.clone();
            if row != rows[ix] {
                rows[ix] = row;
                touched.push(ix);
            }
        }
        let mut last_day = rows.last().and_then(|r| i18n::clock::local_day(r.moment.at));
        let appended = added.len();
        for moment in added {
            let row = day_row(store, moment, me.as_ref(), &audit, now, &mut last_day);
            let ix = rows.len();
            self.index.insert(row.moment.id.clone(), ix);
            if is_stateful(&row) {
                self.stateful.push(ix);
            }
            rows.push(row);
        }
        touched.sort_unstable();
        Some(RowUpdate::Patched { changed: touched, appended })
    }
}

impl Rows {
    /// Adds the task cards that point at a surface moment changed in this
    /// batch to `changed`. A card carries "Show result" and the decision
    /// pointer only once the surface moment exists, and the surface can be
    /// projected after the task change, so the card is rebuilt then. Surfaces
    /// update often, so only the dependent task rows are rebuilt, not the
    /// whole list.
    fn add_surface_dependents<'a>(
        &self,
        store: &'a Store,
        context: &ContextId,
        changes: &ChangeSet,
        changed: &mut Vec<(usize, &'a Moment)>,
    ) {
        let surfaces: Vec<&MomentId> = changes
            .moments()
            .filter(|id| {
                store.moment(id).is_some_and(|m| {
                    &m.context_id == context && matches!(m.kind, MomentKind::AgentSurface(_))
                })
            })
            .collect();
        if surfaces.is_empty() {
            return;
        }
        for (ix, row) in self.rows.iter().enumerate() {
            if !matches!(row.moment.kind, MomentKind::TaskProgress(_))
                || changed.iter().any(|(at, _)| *at == ix)
            {
                continue;
            }
            let Some(moment) = store.moment(&row.moment.id) else { continue };
            let MomentKind::TaskProgress(p) = &moment.kind else { continue };
            let result = store
                .task(&p.task_id)
                .and_then(|t| t.result_ref.as_ref())
                .and_then(|s| store.surface_moment(s));
            let decision = selectors::task_decision_surface(store, moment);
            if [result, decision].into_iter().flatten().any(|id| surfaces.contains(&id)) {
                changed.push((ix, moment));
            }
        }
    }
}

/// A row with its day header when it opens a new local day.
fn day_row(
    store: &Store,
    moment: &Moment,
    me: Option<&ramflux_desktop_core::ParticipantId>,
    audit: &AuditIndex<'_>,
    now: Timestamp,
    last_day: &mut Option<chrono::NaiveDate>,
) -> MomentRow {
    let mut row = MomentRow::build_with(store, moment, me, audit);
    let day = i18n::clock::local_day(moment.at);
    if day != *last_day {
        row.day_header = Some(i18n::day_label(moment.at, now));
        *last_day = day;
    }
    row
}
