// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The admin owner of the MLS groups (MLS GUI batch M3): the admin slot,
//! pending commits, join requests, the push mode, the catch-up record and
//! the warning set of each group of the account.
//!
//! Like the rest of the MLS state it is account-scoped, lives in memory only
//! and outlives every view. Rules:
//!
//! - **One admin commit in flight per group.** An approve, role change,
//!   removal, ban, settings change or void takes the group's slot when the
//!   source takes the call, and keeps it until a typed refusal or until the
//!   event at the committed position arrives. Decline, push and skip have
//!   their own in-flight marks.
//! - **Ids travel with every outcome.** The account epoch, the action id and
//!   the target come back with the result. A result applies only when the
//!   epoch is unchanged, the group is still known and not read only, and the
//!   slot still holds that action; otherwise it is dropped.
//! - **Success needs the reply and the event.** `committed{log_seq}` moves
//!   the action to pending; the row at that position resolves it. The last
//!   64 commit positions per group are kept, so an event that came before
//!   its reply resolves the action at once.
//! - **Late reads never undo newer events.** An info read carries the group's
//!   event counter from when it was issued; a warning or catch-up event
//!   applied after that wins over what the read says. A page of requests
//!   carries the page generation; a page asked before the last reset is
//!   dropped. A skip result applies only while the catch-up sequence it
//!   started from is unchanged.
//! - **Nothing is sent from here.** The follow-ups named here are reads.
//! - **Lock and removal** clear all of it and bump the epoch, which survives
//!   the clearing, so no late result writes back.
//!
//! Logs: none from this file (kind names and codes only elsewhere).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};

use super::Store;
use super::changes::Changes;
use crate::ids::Timestamp;
use crate::model::mls_group::{
    AdminAction, AdminFailure, AdminKind, AdminPending, AdminResult, AdminState, AdminTarget,
    CatchUpRecord, CatchUpState, DeclineResult, JoinRequest, MembershipKind, MlsGroupSummary,
    MlsRole, OpaqueId, Post, PostBody, PushResult, PushSetting, RequestPager, RequestRowState,
    SkipResult, UndoOffer, WarningCode,
};

/// Applied commit positions remembered per group.
const APPLIED_COMMITS: usize = 64;

/// The class of a commit row: which admin actions it can resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CommitClass {
    Membership,
    Settings,
    Void,
}

fn class_of(kind: AdminKind) -> CommitClass {
    match kind {
        AdminKind::Approve | AdminKind::Role | AdminKind::Remove | AdminKind::Ban => {
            CommitClass::Membership
        }
        AdminKind::Settings => CommitClass::Settings,
        AdminKind::Void => CommitClass::Void,
    }
}

/// A page of join requests, as the reducer takes it.
pub(super) struct RequestsPage<'a> {
    pub(super) group: &'a str,
    pub(super) page_gen: u64,
    pub(super) first_page: bool,
    pub(super) rows: &'a [JoinRequest],
    pub(super) cursor: Option<String>,
    pub(super) failed: Option<String>,
}

/// A counter that survives the clearing of the state and clones by value.
#[derive(Debug, Default)]
pub(super) struct Counter(AtomicU64);

impl Clone for Counter {
    fn clone(&self) -> Self {
        Self(AtomicU64::new(self.0.load(Ordering::Relaxed)))
    }
}

/// The banners hidden for the session: view input, like the mention picks,
/// so a view can set it through the shared store.
#[derive(Debug, Default)]
pub(super) struct Hidden(parking_lot::Mutex<BTreeSet<(String, String)>>);

impl Clone for Hidden {
    fn clone(&self) -> Self {
        Self(parking_lot::Mutex::new(self.0.lock().clone()))
    }
}

/// One warning of a group and when an event last touched it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WarningEntry {
    code: WarningCode,
    active: bool,
    touched: u64,
}

/// The admin state of the account's MLS groups.
#[derive(Debug, Clone, Default)]
pub(super) struct AdminOwner {
    epoch: u64,
    counter: Counter,
    slot: BTreeMap<String, AdminAction>,
    pending: BTreeMap<String, AdminPending>,
    applied: BTreeMap<String, VecDeque<(u64, CommitClass)>>,
    paced_until: BTreeMap<String, Timestamp>,
    failure: BTreeMap<String, AdminFailure>,
    requests: BTreeMap<String, RequestPager>,
    declining: BTreeSet<(String, OpaqueId)>,
    push_saving: BTreeMap<String, PushSetting>,
    push_failed: BTreeSet<String>,
    catch_up: BTreeMap<String, CatchUpRecord>,
    warnings: BTreeMap<String, BTreeMap<String, WarningEntry>>,
    event_gen: BTreeMap<String, u64>,
    hidden: Hidden,
    /// Groups whose newest history page is asked after a rejoin.
    pub(super) newest_wanted: BTreeSet<String>,
    /// Groups whose first member page is asked again (after a void).
    pub(super) members_wanted: BTreeSet<String>,
}

impl AdminOwner {
    /// What survives a clearing: the next epoch and the counter.
    pub(super) fn after_clear(&self) -> Self {
        Self { epoch: self.epoch + 1, counter: self.counter.clone(), ..Self::default() }
    }

    /// Forgets everything of one group (it turned read only).
    fn forget(&mut self, group: &str) {
        self.slot.remove(group);
        self.pending.remove(group);
        self.applied.remove(group);
        self.paced_until.remove(group);
        self.failure.remove(group);
        self.requests.remove(group);
        self.declining.retain(|(g, _)| g != group);
        self.push_saving.remove(group);
        self.push_failed.remove(group);
        self.catch_up.remove(group);
        self.warnings.remove(group);
        self.event_gen.remove(group);
        self.hidden.0.lock().retain(|(g, _)| g != group);
        self.newest_wanted.remove(group);
        self.members_wanted.remove(group);
    }

    fn bump(&mut self, group: &str) -> u64 {
        let counter = self.event_gen.entry(group.to_owned()).or_insert(0);
        *counter += 1;
        *counter
    }
}

impl Store {
    // ---- reads ----------------------------------------------------------

    /// The account epoch: it changes whenever the MLS state is cleared (lock,
    /// removal). Every M3 call carries it and its outcome echoes it.
    pub fn mls_epoch(&self) -> u64 {
        self.mls.admin.epoch
    }

    /// A new action id (client-made, from a per-store counter).
    pub fn mls_next_action_id(&self) -> u64 {
        self.mls.admin.counter.0.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// This account's role in a group, from the latest summary.
    pub fn mls_my_role(&self, group_id: &str) -> Option<&MlsRole> {
        self.mls.groups.get(group_id).map(|g| &g.my_role)
    }

    /// Whether this account is an owner or an admin of the group (and the
    /// group takes its calls).
    pub fn mls_is_admin(&self, group_id: &str) -> bool {
        !self.mls.read_only.contains_key(group_id)
            && matches!(self.mls_my_role(group_id), Some(MlsRole::Owner | MlsRole::Admin))
    }

    /// Whether this account is the owner of the group.
    pub fn mls_is_owner(&self, group_id: &str) -> bool {
        !self.mls.read_only.contains_key(group_id)
            && matches!(self.mls_my_role(group_id), Some(MlsRole::Owner))
    }

    /// Where the admin slot of a group stands.
    pub fn mls_admin_state(&self, group_id: &str) -> AdminState<'_> {
        if let Some(action) = self.mls.admin.slot.get(group_id) {
            AdminState::Running(action)
        } else if let Some(pending) = self.mls.admin.pending.get(group_id) {
            AdminState::Updating(pending)
        } else {
            AdminState::Idle
        }
    }

    /// Whether an admin action holds the group.
    pub fn mls_admin_busy(&self, group_id: &str) -> bool {
        !matches!(self.mls_admin_state(group_id), AdminState::Idle)
    }

    /// The action that holds the group, running or waiting for its event.
    pub fn mls_admin_action(&self, group_id: &str) -> Option<&AdminAction> {
        match self.mls_admin_state(group_id) {
            AdminState::Idle => None,
            AdminState::Running(action) => Some(action),
            AdminState::Updating(pending) => Some(&pending.action),
        }
    }

    /// Until when group commits are paced, while that is after `now`.
    pub fn mls_paced_until(&self, group_id: &str, now: Timestamp) -> Option<Timestamp> {
        self.mls.admin.paced_until.get(group_id).copied().filter(|until| until.0 > now.0)
    }

    /// The last typed refusal of an admin action of the group.
    pub fn mls_admin_failure(&self, group_id: &str) -> Option<&AdminFailure> {
        self.mls.admin.failure.get(group_id)
    }

    /// The join requests of a group as far as they are loaded.
    pub fn mls_requests(&self, group_id: &str) -> Option<&RequestPager> {
        self.mls.admin.requests.get(group_id)
    }

    /// Whether a decline of the request is in flight.
    pub fn mls_declining(&self, group_id: &str, request: &OpaqueId) -> bool {
        self.mls.admin.declining.contains(&(group_id.to_owned(), request.clone()))
    }

    /// The mode a push change of the group is saving, while it runs.
    pub fn mls_push_saving(&self, group_id: &str) -> Option<&PushSetting> {
        self.mls.admin.push_saving.get(group_id)
    }

    /// Whether the last push change of the group failed.
    pub fn mls_push_failed(&self, group_id: &str) -> bool {
        self.mls.admin.push_failed.contains(group_id)
    }

    /// The catch-up record of a group.
    pub fn mls_catch_up(&self, group_id: &str) -> Option<&CatchUpRecord> {
        self.mls.admin.catch_up.get(group_id)
    }

    /// The group's event counter: an info read carries it from when it is
    /// issued.
    pub fn mls_event_gen(&self, group_id: &str) -> u64 {
        self.mls.admin.event_gen.get(group_id).copied().unwrap_or(0)
    }

    /// The page generation a first page of requests is asked with (the next
    /// one), or the current one for a later page.
    pub fn mls_requests_gen(&self, group_id: &str, first_page: bool) -> u64 {
        let current = self.mls.admin.requests.get(group_id).map_or(0, |p| p.page_gen);
        if first_page { current + 1 } else { current }
    }

    /// The warnings of a group in banner order, active ones only. A warning
    /// that only owners and admins see is left out for others.
    pub fn mls_warnings(&self, group_id: &str) -> Vec<&WarningCode> {
        let admin = self.mls_is_admin(group_id);
        let mut out: Vec<&WarningCode> = self
            .mls
            .admin
            .warnings
            .get(group_id)
            .into_iter()
            .flat_map(BTreeMap::values)
            .filter(|e| e.active)
            .map(|e| &e.code)
            .filter(|c| admin || !c.admin_only())
            .collect();
        out.sort_by(|a, b| {
            (a.banner_rank().unwrap_or(u8::MAX), a.code())
                .cmp(&(b.banner_rank().unwrap_or(u8::MAX), b.code()))
        });
        out
    }

    /// The warnings that show a banner now: a rank, not hidden for the
    /// session.
    pub fn mls_banner_warnings(&self, group_id: &str) -> Vec<&WarningCode> {
        self.mls_warnings(group_id)
            .into_iter()
            .filter(|c| c.banner_rank().is_some())
            .filter(|c| !self.mls_banner_hidden(group_id, c.code()))
            .collect()
    }

    /// Whether the banner of a code is hidden for the session.
    pub fn mls_banner_hidden(&self, group_id: &str, code: &str) -> bool {
        self.mls.admin.hidden.0.lock().contains(&(group_id.to_owned(), code.to_owned()))
    }

    /// Hides the banner of a code for the session (in memory, cleared on
    /// lock). Only the three hideable codes can be hidden.
    pub fn mls_hide_banner(&self, group_id: &str, code: &str) {
        if self.mls_locked() {
            return;
        }
        let hideable = self
            .mls
            .admin
            .warnings
            .get(group_id)
            .and_then(|w| w.get(code))
            .is_some_and(|e| e.active && e.code.hideable());
        if hideable {
            self.mls.admin.hidden.0.lock().insert((group_id.to_owned(), code.to_owned()));
        }
    }

    /// Whether the group host sees this device's address (the M2 info row):
    /// from the warning set, which events keep current.
    pub fn mls_host_ip_visible(&self, group_id: &str) -> bool {
        self.mls
            .admin
            .warnings
            .get(group_id)
            .is_some_and(|w| w.values().any(|e| e.active && e.code.code() == "host_ip_visible"))
    }

    /// The position of the head commit: the commit row with the greatest log
    /// position of the timeline, while the newest page is loaded and the
    /// window is not cut at the newest end. A void row, or a row a void row
    /// undid, is not offered.
    pub fn mls_head_commit(&self, group_id: &str) -> Option<u64> {
        let tl = self.mls.timelines.get(group_id)?;
        if !tl.loaded || tl.tail_cut {
            return None;
        }
        let (log_seq, post) = tl.posted.iter().rev().find(|(_, p)| {
            matches!(
                p.body,
                PostBody::Membership { .. } | PostBody::Settings { .. } | PostBody::Void { .. }
            )
        })?;
        if matches!(post.body, PostBody::Void { .. }) {
            return None;
        }
        if self.mls_voided(group_id).contains(log_seq) {
            return None;
        }
        Some(*log_seq)
    }

    /// The positions a void row of the timeline undid.
    pub fn mls_voided(&self, group_id: &str) -> BTreeSet<u64> {
        self.mls
            .timelines
            .get(group_id)
            .into_iter()
            .flat_map(|tl| tl.posted.values())
            .filter_map(|p| match &p.body {
                PostBody::Void { voided_log_seq: Some(v), .. } => Some(*v),
                _ => None,
            })
            .collect()
    }

    /// The undo entry of the commit row at `log_seq`.
    pub(super) fn mls_undo_offer(
        &self,
        group_id: &str,
        head: Option<u64>,
        log_seq: u64,
    ) -> UndoOffer {
        if head != Some(log_seq) || !self.mls_is_admin(group_id) {
            return UndoOffer::No;
        }
        match self.mls_admin_action(group_id) {
            None => UndoOffer::Offer { log_seq },
            Some(action)
                if action.kind == AdminKind::Void
                    && action.target == AdminTarget::LogSeq(log_seq) =>
            {
                UndoOffer::Undoing { log_seq }
            }
            Some(_) => UndoOffer::Busy { log_seq },
        }
    }

    // ---- reducer: calls taken ------------------------------------------

    /// A call the source took: the slot, an in-flight mark or a loading flag.
    pub(super) fn mls_admin_started(
        &mut self,
        at: Timestamp,
        start: &crate::event::MlsStart,
        changes: &mut Changes,
    ) {
        use crate::event::MlsStart;
        match start {
            MlsStart::Requests { group_id, first_page, page_gen } => {
                let pager = self.mls.admin.requests.entry(group_id.clone()).or_default();
                if *first_page {
                    *pager = RequestPager {
                        page_gen: *page_gen,
                        loading: true,
                        ..RequestPager::default()
                    };
                } else {
                    pager.loading_more = true;
                    pager.page_error = None;
                }
                changes.mls.insert(group_id.clone());
            }
            MlsStart::Admin { group_id, epoch, action_id, kind, target } => {
                if *epoch != self.mls.admin.epoch
                    || self.mls.read_only.contains_key(group_id)
                    || !self.mls.groups.contains_key(group_id)
                {
                    return;
                }
                self.mls.admin.failure.remove(group_id);
                self.mls.admin.slot.insert(
                    group_id.clone(),
                    AdminAction {
                        action_id: *action_id,
                        kind: *kind,
                        target: target.clone(),
                        issued_epoch: *epoch,
                        issued_at: at,
                    },
                );
                if let (AdminKind::Approve, AdminTarget::Request(id)) = (kind, target) {
                    self.set_request_state(group_id, id, RequestRowState::Approving, None);
                }
                self.mls_reproject(group_id, changes);
            }
            MlsStart::Decline { group_id, request_id } => {
                if self.mls.read_only.contains_key(group_id) {
                    return;
                }
                self.mls.admin.declining.insert((group_id.clone(), request_id.clone()));
                self.set_request_state(group_id, request_id, RequestRowState::Declining, None);
                changes.mls.insert(group_id.clone());
            }
            MlsStart::Push { group_id, push } => {
                self.mls.admin.push_saving.insert(group_id.clone(), push.clone());
                self.mls.admin.push_failed.remove(group_id);
                changes.mls.insert(group_id.clone());
            }
            MlsStart::Skip { group_id, from_seq } => {
                if let Some(record) = self.mls.admin.catch_up.get_mut(group_id) {
                    record.skipping = Some(*from_seq);
                    record.skip_failure = None;
                }
                changes.mls.insert(group_id.clone());
            }
            MlsStart::Members { group_id } => {
                self.mls.admin.members_wanted.remove(group_id);
            }
            _ => {}
        }
    }

    fn set_request_state(
        &mut self,
        group: &str,
        id: &OpaqueId,
        state: RequestRowState,
        failure: Option<String>,
    ) {
        if let Some(row) = self
            .mls
            .admin
            .requests
            .get_mut(group)
            .and_then(|p| p.rows.iter_mut().find(|r| &r.request.request_id == id))
        {
            row.state = state;
            row.failure = failure;
        }
    }

    fn remove_request(&mut self, group: &str, id: &OpaqueId) {
        if let Some(pager) = self.mls.admin.requests.get_mut(group) {
            pager.rows.retain(|r| &r.request.request_id != id);
        }
    }

    // ---- reducer: results ----------------------------------------------

    /// What an admin call came to.
    pub(super) fn mls_admin_outcome(
        &mut self,
        epoch: u64,
        action_id: u64,
        group: &str,
        result: &AdminResult,
        changes: &mut Changes,
    ) {
        if epoch != self.mls.admin.epoch {
            return;
        }
        let holds = self.mls.admin.slot.get(group).is_some_and(|a| a.action_id == action_id);
        if !holds {
            return;
        }
        let Some(action) = self.mls.admin.slot.remove(group) else { return };
        if self.mls.read_only.contains_key(group) || !self.mls.groups.contains_key(group) {
            self.mls.admin.forget(group);
            changes.mls.insert(group.to_owned());
            return;
        }
        match result {
            AdminResult::Committed { log_seq } => {
                self.mls.admin.failure.remove(group);
                // The next group read shows the quota this commit used.
                self.mls.info_wanted.insert(group.to_owned());
                let early = self
                    .mls
                    .admin
                    .applied
                    .get(group)
                    .is_some_and(|q| q.contains(&(*log_seq, class_of(action.kind))));
                if early {
                    self.mls_admin_resolved(group, &action, changes);
                } else {
                    if let (AdminKind::Approve, AdminTarget::Request(id)) =
                        (action.kind, &action.target)
                    {
                        self.set_request_state(group, id, RequestRowState::Adding, None);
                    }
                    self.mls
                        .admin
                        .pending
                        .insert(group.to_owned(), AdminPending { action, log_seq: *log_seq });
                }
            }
            AdminResult::Refused { code, retry_at, head_log_seq } => {
                self.mls.admin.failure.insert(
                    group.to_owned(),
                    AdminFailure { kind: action.kind, code: code.clone() },
                );
                if let (AdminKind::Approve, AdminTarget::Request(id)) =
                    (action.kind, &action.target)
                {
                    if code == "request_not_found" {
                        self.remove_request(group, id);
                    } else {
                        self.set_request_state(
                            group,
                            id,
                            RequestRowState::Idle,
                            Some(code.clone()),
                        );
                    }
                }
                if code == "paced"
                    && let Some(at) = retry_at
                {
                    self.mls.admin.paced_until.insert(group.to_owned(), *at);
                }
                if code == "not_admin" || code == "not_owner" {
                    // The menu follows the real role.
                    self.mls.info_wanted.insert(group.to_owned());
                }
                if head_log_seq.is_some() || code == "void_not_head" {
                    // A gap fill finds the newer commit.
                    if let Some(tl) = self.mls.timelines.get_mut(group)
                        && tl.loaded
                        && !tl.tail_cut
                    {
                        if tl.gap_in_flight {
                            tl.gap_dirty = true;
                        } else {
                            tl.gap_pending = true;
                        }
                    }
                    self.mls.info_wanted.insert(group.to_owned());
                }
            }
        }
        self.mls_reproject(group, changes);
    }

    /// The pending action of a group resolved: its slot is free.
    fn mls_admin_resolved(&mut self, group: &str, action: &AdminAction, changes: &mut Changes) {
        self.mls.admin.pending.remove(group);
        self.mls.admin.failure.remove(group);
        if let (AdminKind::Approve, AdminTarget::Request(id)) = (action.kind, &action.target) {
            self.remove_request(group, id);
        }
        if matches!(action.kind, AdminKind::Void) {
            self.mls.info_wanted.insert(group.to_owned());
            self.mls.admin.members_wanted.insert(group.to_owned());
        }
        changes.mls.insert(group.to_owned());
    }

    /// A commit row arrived (an event or a history page): it is remembered,
    /// and it resolves the pending action it belongs to.
    pub(super) fn mls_admin_on_commit_row(
        &mut self,
        group: &str,
        post: &Post,
        changes: &mut Changes,
    ) {
        let class = match &post.body {
            PostBody::Membership { .. } => CommitClass::Membership,
            PostBody::Settings { .. } => CommitClass::Settings,
            PostBody::Void { .. } => CommitClass::Void,
            _ => return,
        };
        let Some(log_seq) = post.log_seq else { return };
        let queue = self.mls.admin.applied.entry(group.to_owned()).or_default();
        if !queue.contains(&(log_seq, class)) {
            queue.push_back((log_seq, class));
            while queue.len() > APPLIED_COMMITS {
                queue.pop_front();
            }
        }
        let resolves = self
            .mls
            .admin
            .pending
            .get(group)
            .is_some_and(|p| p.log_seq == log_seq && class_of(p.action.kind) == class);
        if resolves && let Some(pending) = self.mls.admin.pending.get(group).cloned() {
            self.mls_admin_resolved(group, &pending.action, changes);
        }
        match &post.body {
            PostBody::Membership { changes: rows, .. } => {
                for change in rows {
                    if change.kind == MembershipKind::Joined {
                        // The request of a member who joined is done.
                        let id = self.mls.admin.requests.get(group).and_then(|p| {
                            p.rows
                                .iter()
                                .find(|r| r.request.anchor.expose() == change.anchor)
                                .map(|r| r.request.request_id.clone())
                        });
                        if let Some(id) = id {
                            self.remove_request(group, &id);
                        }
                    }
                }
            }
            PostBody::Settings { change, .. } => {
                if let Some(summary) = self.mls.groups.get(group).cloned() {
                    let mut next = summary.clone();
                    if let Some(name) = &change.name {
                        next.name.clone_from(name);
                    }
                    if let Some(approval) = change.approval {
                        next.approval = approval;
                        if !approval {
                            self.mls_requests_reset(group);
                            self.mls_warning_set(
                                group,
                                WarningCode::AdmissionNeedsOnline,
                                true,
                                false,
                            );
                        } else {
                            // Approval on: nobody has to be online to admit.
                            self.mls_warning_set(
                                group,
                                WarningCode::AdmissionNeedsOnline,
                                false,
                                false,
                            );
                        }
                    }
                    if next != summary {
                        self.mls_upsert_group(next, changes);
                    }
                }
            }
            _ => {}
        }
        changes.mls.insert(group.to_owned());
    }

    /// Drops the loaded requests of a group and invalidates pages in flight.
    fn mls_requests_reset(&mut self, group: &str) {
        if let Some(pager) = self.mls.admin.requests.get_mut(group) {
            let page_gen = pager.page_gen + 1;
            *pager = RequestPager { page_gen, ..RequestPager::default() };
        }
    }

    /// What a decline came to.
    pub(super) fn mls_decline_outcome(
        &mut self,
        epoch: u64,
        group: &str,
        request: &OpaqueId,
        result: &DeclineResult,
        changes: &mut Changes,
    ) {
        if epoch != self.mls.admin.epoch {
            return;
        }
        if !self.mls.admin.declining.remove(&(group.to_owned(), request.clone())) {
            return;
        }
        match result {
            DeclineResult::Declined => self.remove_request(group, request),
            DeclineResult::Refused { code } if code == "request_not_found" => {
                self.remove_request(group, request);
                self.mls.admin.failure.insert(
                    group.to_owned(),
                    AdminFailure { kind: AdminKind::Approve, code: code.clone() },
                );
            }
            DeclineResult::Refused { code } => {
                self.set_request_state(group, request, RequestRowState::Idle, Some(code.clone()));
            }
        }
        changes.mls.insert(group.to_owned());
    }

    /// What a push-mode change came to.
    pub(super) fn mls_push_outcome(
        &mut self,
        epoch: u64,
        group: &str,
        result: &PushResult,
        changes: &mut Changes,
    ) {
        if epoch != self.mls.admin.epoch {
            return;
        }
        if self.mls.admin.push_saving.remove(group).is_none() {
            return;
        }
        match result {
            PushResult::Set(push) => {
                self.mls.admin.push_failed.remove(group);
                if let Some(summary) = self.mls.groups.get(group).cloned() {
                    let mut next = summary.clone();
                    next.push = push.clone();
                    if next != summary {
                        self.mls_upsert_group(next, changes);
                    }
                }
            }
            PushResult::Refused { .. } => {
                self.mls.admin.push_failed.insert(group.to_owned());
            }
        }
        changes.mls.insert(group.to_owned());
    }

    /// What a skip came to: it applies only while the catch-up state it
    /// started from is the one the group has.
    pub(super) fn mls_skip_outcome(
        &mut self,
        epoch: u64,
        group: &str,
        from_seq: u64,
        result: &SkipResult,
        changes: &mut Changes,
    ) {
        if epoch != self.mls.admin.epoch || self.mls.read_only.contains_key(group) {
            return;
        }
        let Some(record) = self.mls.admin.catch_up.get(group) else { return };
        if record.skipping != Some(from_seq) {
            return;
        }
        let late = record.seq != from_seq;
        if let Some(record) = self.mls.admin.catch_up.get_mut(group) {
            record.skipping = None;
        }
        if late {
            // A newer catch-up state was applied since: nothing changes.
            changes.mls.insert(group.to_owned());
            return;
        }
        match result {
            SkipResult::Rejoining => {
                self.mls_catch_up_apply(group, CatchUpState::Rejoining, 0, 0);
            }
            SkipResult::Running { done, total } => {
                self.mls_catch_up_apply(group, CatchUpState::Running, *done, *total);
            }
            SkipResult::Refused { code } => {
                if let Some(record) = self.mls.admin.catch_up.get_mut(group) {
                    record.skip_failure = Some(code.clone());
                }
            }
        }
        changes.mls.insert(group.to_owned());
    }

    /// A page of join requests, or its failure.
    pub(super) fn mls_requests_page(&mut self, page: RequestsPage<'_>, changes: &mut Changes) {
        let RequestsPage { group, page_gen, first_page, rows, cursor, failed } = page;
        let Some(pager) = self.mls.admin.requests.get(group) else { return };
        if pager.page_gen != page_gen || self.mls.read_only.contains_key(group) {
            return;
        }
        for row in rows {
            if row.ai_flag {
                self.mls.ai_true.insert((group.to_owned(), row.anchor.expose().to_owned()));
            }
        }
        let Some(pager) = self.mls.admin.requests.get_mut(group) else { return };
        match failed {
            Some(code) => {
                if first_page {
                    pager.loading = false;
                    pager.error = Some(code);
                } else {
                    pager.loading_more = false;
                    pager.page_error = Some(code);
                }
            }
            None => {
                if first_page {
                    pager.loading = false;
                    pager.loaded = true;
                    pager.error = None;
                    // Rows that were added by events while the page was on its
                    // way stay; the page's rows join them.
                } else {
                    pager.loading_more = false;
                    pager.page_error = None;
                }
                pager.cursor = cursor;
                for row in rows {
                    pager.insert(row.clone());
                }
            }
        }
        changes.mls.insert(group.to_owned());
    }

    /// A join request arrived: it joins the loaded list of an admin of an
    /// approval group, once per request id.
    pub(super) fn mls_pending_join(
        &mut self,
        group: &str,
        request: &JoinRequest,
        changes: &mut Changes,
    ) {
        let approval = self.mls.groups.get(group).is_some_and(|g| g.approval);
        if !approval || !self.mls_is_admin(group) {
            return;
        }
        if request.ai_flag {
            self.mls.ai_true.insert((group.to_owned(), request.anchor.expose().to_owned()));
        }
        if let Some(pager) = self.mls.admin.requests.get_mut(group)
            && (pager.loaded || pager.loading)
            && pager.insert(request.clone())
        {
            changes.mls.insert(group.to_owned());
        }
    }

    // ---- catch-up and warnings -----------------------------------------

    /// Applies a catch-up state (an event, a skip result or an info read).
    /// A different state gets a new sequence number; a `rejoining → idle`
    /// move asks for the newest history page and a group read.
    fn mls_catch_up_apply(&mut self, group: &str, state: CatchUpState, done: u64, total: u64) {
        let previous = self.mls.admin.catch_up.get(group).map(|r| r.state);
        let changed = previous.is_none_or(|p| p != state);
        if changed {
            let seq = self.mls.admin.bump(group);
            let record = self.mls.admin.catch_up.entry(group.to_owned()).or_insert(CatchUpRecord {
                state,
                done,
                total,
                seq,
                skipping: None,
                skip_failure: None,
            });
            record.state = state;
            record.done = done;
            record.total = total;
            record.seq = seq;
            if state != CatchUpState::Running {
                record.skip_failure = None;
            }
        } else if let Some(record) = self.mls.admin.catch_up.get_mut(group) {
            record.done = done;
            record.total = total;
        }
        if previous == Some(CatchUpState::Rejoining) && state == CatchUpState::Idle {
            self.mls.admin.newest_wanted.insert(group.to_owned());
            self.mls.info_wanted.insert(group.to_owned());
        }
    }

    /// A `catch_up` event.
    pub(super) fn mls_catch_up_event(
        &mut self,
        group: &str,
        state: CatchUpState,
        done: u64,
        total: u64,
        changes: &mut Changes,
    ) {
        if self.mls.read_only.contains_key(group) {
            return;
        }
        self.mls_catch_up_apply(group, state, done, total);
        changes.mls.insert(group.to_owned());
    }

    /// Sets or clears one warning. An event touches the code at a new
    /// counter value; a seeded one (`from_event` false) keeps what it had.
    fn mls_warning_set(&mut self, group: &str, code: WarningCode, active: bool, from_event: bool) {
        let bumped = if from_event { self.mls.admin.bump(group) } else { 0 };
        let key = code.code().to_owned();
        let entries = self.mls.admin.warnings.entry(group.to_owned()).or_default();
        let was_active = entries.get(&key).is_some_and(|e| e.active);
        let touched = if from_event { bumped } else { entries.get(&key).map_or(0, |e| e.touched) };
        entries.insert(key.clone(), WarningEntry { code, active, touched });
        if !active || !was_active {
            // A banner hidden for the session comes back with a new warning.
            self.mls.admin.hidden.0.lock().remove(&(group.to_owned(), key));
        }
    }

    /// A `warning` event.
    pub(super) fn mls_warning_event(
        &mut self,
        group: &str,
        code: &WarningCode,
        cleared: bool,
        changes: &mut Changes,
    ) {
        if self.mls.read_only.contains_key(group) {
            return;
        }
        self.mls_warning_set(group, code.clone(), !cleared, true);
        self.mls_reproject(group, changes);
    }

    /// Seeds the warning set and the catch-up record of a group from the
    /// rows of a list read, once; events and info reads keep them current.
    pub(super) fn mls_warnings_seed(&mut self, summary: &MlsGroupSummary) {
        if let Some(c) = &summary.catch_up {
            let state = CatchUpState::parse(&c.state);
            if state != CatchUpState::Idle
                && !self.mls.admin.catch_up.contains_key(&summary.group_id)
            {
                self.mls_catch_up_apply(&summary.group_id, state, c.done, c.total);
            }
        }
        if self.mls.admin.warnings.contains_key(&summary.group_id) {
            return;
        }
        let mut entries = BTreeMap::new();
        for w in &summary.warnings {
            let code = WarningCode::parse(&w.code, &w.params);
            entries.insert(
                code.code().to_owned(),
                WarningEntry { code, active: !w.cleared, touched: 0 },
            );
        }
        self.mls.admin.warnings.insert(summary.group_id.clone(), entries);
    }

    /// Merges a `group.info` read: warnings and catch-up state apply only
    /// where no event was applied after the read was issued.
    pub(super) fn mls_info_merge(
        &mut self,
        summary: &MlsGroupSummary,
        issued: u64,
        changes: &mut Changes,
    ) {
        let group = summary.group_id.as_str();
        if self.mls.read_only.contains_key(group) {
            return;
        }
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for w in &summary.warnings {
            let code = WarningCode::parse(&w.code, &w.params);
            let key = code.code().to_owned();
            seen.insert(key.clone());
            let entries = self.mls.admin.warnings.entry(group.to_owned()).or_default();
            if entries.get(&key).is_some_and(|e| e.touched > issued) {
                continue;
            }
            let touched = entries.get(&key).map_or(0, |e| e.touched);
            entries.insert(key, WarningEntry { code, active: !w.cleared, touched });
        }
        if let Some(entries) = self.mls.admin.warnings.get_mut(group) {
            for (key, entry) in entries.iter_mut() {
                if entry.active && !seen.contains(key) && entry.touched <= issued {
                    entry.active = false;
                }
            }
        }
        self.mls_prune_hidden(group);
        let newer = self.mls.admin.catch_up.get(group).is_some_and(|r| r.seq > issued);
        if !newer {
            match &summary.catch_up {
                Some(c) => {
                    let state = CatchUpState::parse(&c.state);
                    if state != CatchUpState::Idle || self.mls.admin.catch_up.contains_key(group) {
                        self.mls_catch_up_apply(group, state, c.done, c.total);
                    }
                }
                None => {
                    if self.mls.admin.catch_up.contains_key(group) {
                        self.mls_catch_up_apply(group, CatchUpState::Idle, 0, 0);
                    }
                }
            }
        }
        let approval_off = !summary.approval;
        if approval_off {
            self.mls_requests_reset(group);
        }
        changes.mls.insert(group.to_owned());
    }

    /// Drops the hide marks of banners whose warning is no longer active.
    fn mls_prune_hidden(&mut self, group: &str) {
        let active: BTreeSet<String> = self
            .mls
            .admin
            .warnings
            .get(group)
            .into_iter()
            .flat_map(BTreeMap::iter)
            .filter(|(_, e)| e.active)
            .map(|(k, _)| k.clone())
            .collect();
        self.mls.admin.hidden.0.lock().retain(|(g, c)| g != group || active.contains(c));
    }

    /// Forgets the admin state of a group that turned read only.
    pub(super) fn mls_admin_forget(&mut self, group: &str) {
        self.mls.admin.forget(group);
    }
}
