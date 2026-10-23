// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Secondary indexes, maintained by the reducer together with the
//! collections they index (never by anyone else).
//!
//! They make selectors cost O(k) in the size of their result instead of a
//! scan over every moment: a context's timeline, the "needs me" items, the
//! moments in progress, the contexts a participant belongs to, and which
//! moments present a grant or a task.

use std::collections::{BTreeMap, BTreeSet};

use crate::ids::{ContextId, GrantId, MomentId, ParticipantId, TaskId, Timestamp};
use crate::model::{Lifecycle, Moment, MomentKind, Task};

/// The indexes of one store.
#[derive(Debug, Clone, Default)]
pub(crate) struct Indexes {
    /// Moments of each context, ordered by `(at, id)`.
    pub timeline: BTreeMap<ContextId, BTreeSet<(Timestamp, MomentId)>>,
    /// `NeedsMe` moments, ordered by `(since, id)`.
    pub needs_me: BTreeSet<(Timestamp, MomentId)>,
    /// `NeedsMe` moments per context.
    pub needs_me_by_context: BTreeMap<ContextId, BTreeSet<MomentId>>,
    /// `InProgress` moments.
    pub in_progress: BTreeSet<MomentId>,
    /// Contexts each participant is a member of.
    pub contexts_by_participant: BTreeMap<ParticipantId, BTreeSet<ContextId>>,
    /// Moments presenting a grant (an intervention, a surface approval block).
    pub grant_presenters: BTreeMap<GrantId, BTreeSet<MomentId>>,
    /// Task progress moments presenting a task.
    pub task_presenters: BTreeMap<TaskId, BTreeSet<MomentId>>,
    /// Tasks with a step waiting on a grant.
    pub step_owners: BTreeMap<GrantId, BTreeSet<TaskId>>,
}

/// Grants a moment payload presents.
pub(crate) fn presented_grants(kind: &MomentKind) -> Vec<GrantId> {
    match kind {
        MomentKind::Intervention(p) => vec![p.grant_id.clone()],
        MomentKind::AgentSurface(s) => s
            .blocks
            .iter()
            .filter_map(|b| b.data.get("grant_id").and_then(|v| v.as_str()))
            .map(GrantId::new)
            .collect(),
        _ => Vec::new(),
    }
}

fn presented_task(kind: &MomentKind) -> Option<&TaskId> {
    match kind {
        MomentKind::TaskProgress(p) => Some(&p.task_id),
        _ => None,
    }
}

fn remove_from<K: Ord, V: Ord>(map: &mut BTreeMap<K, BTreeSet<V>>, key: &K, value: &V) {
    if let Some(set) = map.get_mut(key) {
        set.remove(value);
        if set.is_empty() {
            map.remove(key);
        }
    }
}

impl Indexes {
    /// Indexes a new moment; `since` is when it entered `NeedsMe`.
    pub fn add_moment(&mut self, moment: &Moment, since: Option<Timestamp>) {
        let id = &moment.id;
        self.timeline.entry(moment.context_id.clone()).or_default().insert((moment.at, id.clone()));
        self.enter(moment, moment.lifecycle, since);
        self.present(id, &moment.kind);
    }

    /// Removes a moment from every index; `since` as recorded.
    pub fn remove_moment(&mut self, moment: &Moment, since: Option<Timestamp>) {
        let id = &moment.id;
        if let Some(set) = self.timeline.get_mut(&moment.context_id) {
            set.remove(&(moment.at, id.clone()));
        }
        self.leave(moment, moment.lifecycle, since);
        self.unpresent(id, &moment.kind);
    }

    /// Moves a moment from lifecycle `from` (entered at `old_since`) to
    /// `to` (entered at `new_since`).
    pub fn change_lifecycle(
        &mut self,
        moment: &Moment,
        (from, old_since): (Lifecycle, Option<Timestamp>),
        (to, new_since): (Lifecycle, Option<Timestamp>),
    ) {
        self.leave(moment, from, old_since);
        self.enter(moment, to, new_since);
    }

    /// Re-indexes what a moment presents after its payload changed.
    pub fn change_kind(&mut self, id: &MomentId, old: &MomentKind, new: &MomentKind) {
        self.unpresent(id, old);
        self.present(id, new);
    }

    /// Re-indexes a context's members.
    pub fn change_members(
        &mut self,
        context: &ContextId,
        old: &[ParticipantId],
        new: &[ParticipantId],
    ) {
        for member in old {
            remove_from(&mut self.contexts_by_participant, member, context);
        }
        for member in new {
            self.contexts_by_participant.entry(member.clone()).or_default().insert(context.clone());
        }
    }

    /// Drops a removed context's own entries (its moments go one by one).
    pub fn remove_context(&mut self, context: &ContextId, members: &[ParticipantId]) {
        self.change_members(context, members, &[]);
        self.timeline.remove(context);
        self.needs_me_by_context.remove(context);
    }

    /// Re-indexes the grants a task's steps wait on.
    pub fn change_task(&mut self, old: Option<&Task>, new: &Task) {
        if let Some(old) = old {
            for grant in old.step_grants() {
                remove_from(&mut self.step_owners, grant, &old.id);
            }
        }
        for grant in new.step_grants() {
            self.step_owners.entry(grant.clone()).or_default().insert(new.id.clone());
        }
    }

    fn enter(&mut self, moment: &Moment, lifecycle: Lifecycle, since: Option<Timestamp>) {
        match lifecycle {
            Lifecycle::NeedsMe => {
                let since = since.unwrap_or(moment.at);
                self.needs_me.insert((since, moment.id.clone()));
                self.needs_me_by_context
                    .entry(moment.context_id.clone())
                    .or_default()
                    .insert(moment.id.clone());
            }
            Lifecycle::InProgress => {
                self.in_progress.insert(moment.id.clone());
            }
            _ => {}
        }
    }

    fn leave(&mut self, moment: &Moment, lifecycle: Lifecycle, since: Option<Timestamp>) {
        match lifecycle {
            Lifecycle::NeedsMe => {
                let since = since.unwrap_or(moment.at);
                self.needs_me.remove(&(since, moment.id.clone()));
                remove_from(&mut self.needs_me_by_context, &moment.context_id, &moment.id);
            }
            Lifecycle::InProgress => {
                self.in_progress.remove(&moment.id);
            }
            _ => {}
        }
    }

    fn present(&mut self, id: &MomentId, kind: &MomentKind) {
        for grant in presented_grants(kind) {
            self.grant_presenters.entry(grant).or_default().insert(id.clone());
        }
        if let Some(task) = presented_task(kind) {
            self.task_presenters.entry(task.clone()).or_default().insert(id.clone());
        }
    }

    fn unpresent(&mut self, id: &MomentId, kind: &MomentKind) {
        for grant in presented_grants(kind) {
            remove_from(&mut self.grant_presenters, &grant, id);
        }
        if let Some(task) = presented_task(kind) {
            remove_from(&mut self.task_presenters, task, id);
        }
    }
}
