// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The MLS groups of the account: the list, members, invites, the join
//! owner, the send owner and the timelines.
//!
//! Everything here is account-scoped, lives in memory only and outlives
//! every view. A locked account (and lock or removal itself) clears it, and
//! while the bound account is locked the reducer refuses every write. Each
//! outcome event carries the ids it applies to (the join attempt, the
//! `local_id` or `post_id`, the `group_id`); an event for another target
//! changes nothing. Nothing is sent or resent from here: the follow-ups
//! this module names (a gap fill, a list read, an info read) are reads, and
//! each is asked once per need.
//!
//! A timeline is projected into moments of an `mls:<group_id>` context, so
//! the conversation view and its bubbles are reused. A moment whose id
//! changes (a post that gets its log position, a collapsed run that grows)
//! is removed and added again in the same step, so two moments never exist
//! for one post. The AI marker is merged here (S8): once an SDK row said an
//! anchor is an AI in a group, it stays one for the session.

use std::collections::{BTreeMap, BTreeSet};

use super::Store;
use super::changes::{ApplyError, Changes};
use crate::event::{DomainEvent, MlsStart};
use crate::ids::{ContextId, MomentId, ParticipantId, Timestamp};
use crate::intent::IntentKind;
use crate::model::mls_group::{
    self as mls, Author, CreateState, DeleteResult, GroupEventKind, HistoryDirection, JoinOutcome,
    JoinRecord, JoinRefusal, JoinState, LeaveResult, ListPhase, ListState, MemberPages, MemberRow,
    MembershipChange, MembershipKind, MlsGroupSummary, MlsName, MlsRole, MlsServiceLine,
    MlsTimeline, NotSentReason, OwnItem, Post, PostBody, PostKind, PostState, QueueReason,
    ReadOnly, SendState,
};
use crate::model::{
    Context, ContextKind, FallbackPayload, IdentityState, KindClaim, Lifecycle, MessagePayload,
    Moment, MomentKind, NameWarnings, OrderKey, Participant, ParticipantKind, Receipt, ReplyRef,
};

/// Event frames remembered for deduplication by `seq`.
const SEEN_SEQ: usize = 512;
/// Smallest gap between two info reads of a group caused by events.
const INFO_COALESCE_MS: i64 = 1_000;

/// The MLS state of one account.
#[derive(Debug, Clone, Default)]
pub(crate) struct MlsState {
    groups: BTreeMap<String, MlsGroupSummary>,
    list: ListState,
    stale: BTreeSet<String>,
    members: BTreeMap<String, MemberPages>,
    invites: BTreeMap<String, Vec<mls::Invite>>,
    join: JoinState,
    timelines: BTreeMap<String, MlsTimeline>,
    projected: BTreeMap<String, BTreeMap<MomentId, Moment>>,
    ai_true: BTreeSet<(String, String)>,
    read_only: BTreeMap<String, ReadOnly>,
    my_anchor: Option<String>,
    create: CreateState,
    failures: BTreeMap<(String, Option<String>), String>,
    refreshed_at: BTreeMap<String, Timestamp>,
    info_at: BTreeMap<String, Timestamp>,
    info_wanted: BTreeSet<String>,
    list_wanted: bool,
    pending_wanted: bool,
    activity: BTreeMap<String, Timestamp>,
    seen_seq: BTreeSet<u64>,
    leave_failure: BTreeMap<String, LeaveResult>,
    leaving: BTreeSet<String>,
    delete_failure: BTreeMap<String, String>,
    unavailable: Option<String>,
    tick: u64,
    clock: Timestamp,
    /// Own posts of evicted timelines that are still submitting or were not
    /// sent: the send owner keeps them for the session (S4).
    parked: BTreeMap<String, Vec<OwnItem>>,
    /// Until when the daily limit of a group is used up.
    rate_locks: BTreeMap<String, Timestamp>,
}

/// The author of a post: its `author`, else the actor of a service row.
fn actor(post: &Post) -> Option<&Author> {
    post.author.as_ref().or(match &post.body {
        PostBody::Membership { by, .. }
        | PostBody::Settings { by, .. }
        | PostBody::Void { by, .. } => by.as_ref(),
        _ => None,
    })
}

fn name_of(author: &Author) -> MlsName {
    MlsName { display: author.display.clone(), is_me: author.is_me }
}

fn change_name(change: &MembershipChange, my: Option<&str>) -> MlsName {
    MlsName {
        display: change.display.clone(),
        is_me: change.is_me || my.is_some_and(|a| a == change.anchor),
    }
}

fn fnv(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

fn participant_of(group: &str, anchor: &str) -> ParticipantId {
    ParticipantId::new(format!(
        "{}{:016x}",
        mls::PARTICIPANT_PREFIX,
        fnv(&format!("{group}\0{anchor}"))
    ))
}

fn service_participant() -> ParticipantId {
    ParticipantId::new(format!("{}service", mls::PARTICIPANT_PREFIX))
}

fn mls_participant(id: ParticipantId, display: &str, ai: bool) -> Participant {
    Participant {
        id,
        kind: ParticipantKind::Human,
        display: display.to_owned(),
        identity: IdentityState::Tofu,
        pending_change: None,
        ai_flag: ai,
        contact: false,
        link: crate::model::ContactLink::None,
        reachable: None,
        kind_claim: KindClaim::None,
        name_warnings: NameWarnings::default(),
        root_source: None,
    }
}

fn key_posted(log_seq: u64) -> OrderKey {
    OrderKey { hlc: log_seq, author: String::new(), device_sequence: 0 }
}

fn key_own(order: u64) -> OrderKey {
    OrderKey { hlc: u64::MAX, author: String::new(), device_sequence: order }
}

/// What an unavailable refusal names.
#[derive(Clone, Copy)]
struct Unavailable<'a> {
    method: &'a str,
    group: Option<&'a str>,
    attempt: Option<u64>,
    local_id: Option<&'a str>,
}

/// A page of history, as the reducer takes it.
struct HistoryPage<'a> {
    group: &'a str,
    direction: HistoryDirection,
    posts: &'a [Post],
    floor: Option<u64>,
    last: u64,
    more: bool,
}

/// A line the projection makes of a membership or settings row.
enum Line {
    Joined(Vec<MlsName>, u64),
    Left(Vec<MlsName>, u64),
    Single(MlsServiceLine, String, u64),
}

impl Store {
    // ---- reads ----------------------------------------------------------

    /// The MLS groups in list order: last activity (newest first), then id.
    pub fn mls_groups(&self) -> Vec<&MlsGroupSummary> {
        let mut rows: Vec<&MlsGroupSummary> = self.mls.groups.values().collect();
        rows.sort_by(|a, b| {
            b.last_activity_at.cmp(&a.last_activity_at).then_with(|| a.group_id.cmp(&b.group_id))
        });
        rows
    }

    /// One MLS group.
    pub fn mls_group(&self, group_id: &str) -> Option<&MlsGroupSummary> {
        self.mls.groups.get(group_id)
    }

    /// Where the group list stands.
    pub fn mls_list(&self) -> &ListState {
        &self.mls.list
    }

    /// The members loaded of a group.
    pub fn mls_members(&self, group_id: &str) -> Option<&MemberPages> {
        self.mls.members.get(group_id)
    }

    /// The invites of a group this account created (held in memory only).
    pub fn mls_invites(&self, group_id: &str) -> &[mls::Invite] {
        self.mls.invites.get(group_id).map_or(&[], Vec::as_slice)
    }

    /// The join owner.
    pub fn mls_join(&self) -> &JoinState {
        &self.mls.join
    }

    /// The attempt number the join sheet takes next.
    pub fn mls_next_attempt(&self) -> u64 {
        self.mls.join.last_attempt + 1
    }

    /// The timeline of a group, while it is kept.
    pub fn mls_timeline(&self, group_id: &str) -> Option<&MlsTimeline> {
        self.mls.timelines.get(group_id)
    }

    /// Why a group takes no posts from this account.
    pub fn mls_read_only(&self, group_id: &str) -> Option<ReadOnly> {
        self.mls.read_only.get(group_id).copied()
    }

    /// Whether the SDK said at some point that `anchor` is an AI in `group_id`.
    pub fn mls_is_ai(&self, group_id: &str, anchor: &str) -> bool {
        self.mls.ai_true.contains(&(group_id.to_owned(), anchor.to_owned()))
    }

    /// The create owner.
    pub fn mls_create(&self) -> &CreateState {
        &self.mls.create
    }

    /// The error code of the last failure of `method` for `group_id`.
    pub fn mls_failure(&self, method: &str, group_id: Option<&str>) -> Option<&str> {
        self.mls.failures.get(&(method.to_owned(), group_id.map(str::to_owned))).map(String::as_str)
    }

    /// Why the last leave of a group did not take effect.
    pub fn mls_leave_failure(&self, group_id: &str) -> Option<&LeaveResult> {
        self.mls.leave_failure.get(group_id)
    }

    /// Whether a leave of the group is in flight.
    pub fn mls_leaving(&self, group_id: &str) -> bool {
        self.mls.leaving.contains(group_id)
    }

    /// The error code of the last delete of `post_id` that did not take effect.
    pub fn mls_delete_failure(&self, post_id: &str) -> Option<&str> {
        self.mls.delete_failure.get(post_id).map(String::as_str)
    }

    /// The method that was refused as unavailable last, when one was.
    pub fn mls_unavailable_method(&self) -> Option<&str> {
        self.mls.unavailable.as_deref()
    }

    /// The own post `local_id` of a group, while it is kept.
    pub fn mls_own_item(&self, group_id: &str, local_id: &str) -> Option<&OwnItem> {
        self.mls.timelines.get(group_id)?.own.iter().find(|i| i.local_id == local_id)
    }

    /// The own posts of a group that were not sent, oldest first.
    pub fn mls_not_sent(&self, group_id: &str) -> Vec<&OwnItem> {
        self.mls.timelines.get(group_id).map_or_else(Vec::new, |t| {
            t.own.iter().filter(|i| matches!(i.send, SendState::NotSent(_))).collect()
        })
    }

    /// Whether `group.refresh` may be sent for `group_id` at `now`.
    pub fn mls_refresh_due(&self, group_id: &str, now: Timestamp) -> bool {
        self.mls
            .refreshed_at
            .get(group_id)
            .is_none_or(|at| now.0.saturating_sub(at.0) >= mls::REFRESH_COALESCE_MS)
    }

    /// Until when the daily limit of the group is used up, while it is at
    /// `now`: the composer is off (and Retry with it) until then. Reading it
    /// sends nothing; a timer that lifts the lock only redraws.
    pub fn mls_rate_locked_until(&self, group_id: &str, now: Timestamp) -> Option<Timestamp> {
        self.mls.rate_locks.get(group_id).copied().filter(|until| until.0 > now.0)
    }

    /// The account's own anchor, once any row said which one it is.
    pub fn mls_my_anchor(&self) -> Option<&str> {
        self.mls.my_anchor.as_deref()
    }

    /// The reads this state asks for, each once per need: a gap fill, the
    /// group list after a join, the info of a group after a membership or
    /// settings event (at most once a second). Marking them asked is the
    /// source's `MlsStarted` event.
    pub fn mls_followups(&self, now: Timestamp) -> Vec<IntentKind> {
        if self.mls_locked() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (group, tl) in &self.mls.timelines {
            if tl.loaded && tl.gap_pending && !tl.gap_in_flight && !tl.tail_cut {
                out.push(IntentKind::MlsGroupHistory {
                    group_id: group.clone(),
                    direction: HistoryDirection::After(tl.last_log_seq),
                });
            }
        }
        if self.mls.list_wanted {
            out.push(IntentKind::MlsGroupList { cursor: None });
        }
        if self.mls.pending_wanted {
            out.push(IntentKind::MlsJoinPending);
        }
        for group in &self.mls.info_wanted {
            let recent = self
                .mls
                .info_at
                .get(group)
                .is_some_and(|at| now.0.saturating_sub(at.0) < INFO_COALESCE_MS);
            if !recent {
                out.push(IntentKind::MlsGroupInfo { group_id: group.clone() });
            }
        }
        out
    }

    pub(crate) fn mls_locked(&self) -> bool {
        self.bound_account
            .as_ref()
            .and_then(|id| self.accounts.get(id))
            .is_some_and(|a| a.lock_state == crate::model::LockState::Locked)
    }

    // ---- clearing -------------------------------------------------------

    /// Forgets every MLS record, its contexts, moments and participants.
    pub(super) fn mls_clear(&mut self, changes: &mut Changes) {
        if self.mls.groups.is_empty()
            && self.mls.timelines.is_empty()
            && self.mls.projected.is_empty()
            && self.mls.join.records.is_empty()
            && self.mls.join.last.is_none()
        {
            self.mls = MlsState::default();
            return;
        }
        let groups: BTreeSet<String> = self
            .contexts
            .keys()
            .filter_map(|c| mls::group_of(c).map(str::to_owned))
            .chain(self.mls.projected.keys().cloned())
            .collect();
        for group in &groups {
            if let Some(ids) = self.mls.projected.remove(group) {
                for id in ids.keys() {
                    self.remove_projected(id, changes);
                }
            }
            let context = mls::context_id(group);
            if self.contexts.contains_key(&context) {
                let _ = self.remove_context(&context, changes);
            }
        }
        let people: Vec<ParticipantId> = self
            .participants
            .keys()
            .filter(|p| p.as_str().starts_with(mls::PARTICIPANT_PREFIX))
            .cloned()
            .collect();
        for person in people {
            self.participants.remove(&person);
            changes.participants.insert(person);
        }
        self.mls = MlsState::default();
        changes.mls.insert(String::new());
        for group in groups {
            changes.mls.insert(group);
        }
    }

    fn remove_projected(&mut self, id: &MomentId, changes: &mut Changes) {
        if let Some(moment) = self.moments.remove(id) {
            let since = self.needs_me_since.remove(id);
            self.index.remove_moment(&moment, since);
            self.bodies.remove(id);
            changes.moments.insert(id.clone());
        }
    }

    // ---- reducer --------------------------------------------------------

    pub(super) fn mls_reduce(
        &mut self,
        at: Timestamp,
        event: &DomainEvent,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        if self.mls_locked() {
            // A locked account takes no MLS write (R4-F2).
            return Ok(());
        }
        self.mls.clock = at;
        match event {
            DomainEvent::MlsStarted(start) => self.mls_started(at, start, changes),
            DomainEvent::MlsGroupsListed { rows, cursor, first_page } => {
                self.mls_listed(rows, cursor.clone(), *first_page, changes);
            }
            DomainEvent::MlsFailed { method, group_id, invite_id, code } => {
                self.mls_failed(method, group_id.as_deref(), invite_id.as_deref(), code, changes);
            }
            DomainEvent::MlsGroupInfoRead { summary } => {
                self.mls.failures.remove(&(
                    crate::intent::methods::MLS_GROUP_INFO.to_owned(),
                    Some(summary.group_id.clone()),
                ));
                self.mls_upsert_group((**summary).clone(), changes);
            }
            DomainEvent::MlsMembersPage { group_id, rows, cursor, failed } => {
                self.mls_members_page(group_id, rows, cursor.clone(), failed.clone(), changes);
            }
            DomainEvent::MlsHistoryPage {
                group_id,
                direction,
                posts,
                floor_log_seq,
                last_log_seq,
                more,
            } => {
                let page = HistoryPage {
                    group: group_id,
                    direction: *direction,
                    posts,
                    floor: *floor_log_seq,
                    last: *last_log_seq,
                    more: *more,
                };
                self.mls_history_page(&page, changes);
            }
            DomainEvent::MlsGroupEvent { seq, group_id, kind, .. } => {
                self.mls_group_event(at, *seq, group_id, kind, changes);
            }
            DomainEvent::MlsCreated { group_id, name, approval, invite } => {
                self.mls.create.in_flight = false;
                self.mls.create.seq += 1;
                self.mls.create.outcome = Some(Ok(group_id.clone()));
                if !self.mls.groups.contains_key(group_id) {
                    let summary = placeholder_summary(group_id, name, *approval, at);
                    self.mls_upsert_group(summary, changes);
                }
                self.mls.invites.insert(group_id.clone(), vec![invite.clone()]);
                self.mls.my_anchor.get_or_insert_with(String::new);
                changes.mls.insert(String::new());
                changes.mls.insert(group_id.clone());
            }
            DomainEvent::MlsCreateFailed { code } => {
                self.mls.create.in_flight = false;
                self.mls.create.seq += 1;
                self.mls.create.outcome = Some(Err(code.clone()));
                changes.mls.insert(String::new());
            }
            DomainEvent::MlsInviteCreated { group_id, invite } => {
                let list = self.mls.invites.entry(group_id.clone()).or_default();
                list.retain(|i| i.invite_id != invite.invite_id);
                list.insert(0, invite.clone());
                self.mls.failures.remove(&(
                    crate::intent::methods::MLS_GROUP_INVITE_CREATE.to_owned(),
                    Some(group_id.clone()),
                ));
                changes.mls.insert(group_id.clone());
            }
            DomainEvent::MlsInvitesListed { group_id, invites } => {
                self.mls.invites.insert(group_id.clone(), invites.clone());
                self.mls.failures.remove(&(
                    crate::intent::methods::MLS_GROUP_INVITE_LIST.to_owned(),
                    Some(group_id.clone()),
                ));
                changes.mls.insert(group_id.clone());
            }
            DomainEvent::MlsInviteRevoked { group_id, invite_id } => {
                if let Some(invite) = self
                    .mls
                    .invites
                    .get_mut(group_id)
                    .and_then(|l| l.iter_mut().find(|i| &i.invite_id == invite_id))
                {
                    invite.revoked = true;
                }
                self.mls.failures.remove(&(
                    crate::intent::methods::MLS_GROUP_INVITE_REVOKE.to_owned(),
                    Some(group_id.clone()),
                ));
                changes.mls.insert(group_id.clone());
            }
            DomainEvent::MlsJoinOutcome { attempt, outcome } => {
                self.mls_join_outcome(*attempt, outcome, changes);
            }
            DomainEvent::MlsJoinPendingListed { records } => {
                let known: BTreeMap<String, u64> =
                    self.mls.join.records.iter().map(|(id, r)| (id.clone(), r.attempt)).collect();
                self.mls.join.records = records
                    .iter()
                    .map(|r| {
                        let mut record = r.clone();
                        record.attempt = known.get(&r.request_id).copied().unwrap_or(r.attempt);
                        (r.request_id.clone(), record)
                    })
                    .collect();
                self.mls.pending_wanted = false;
                changes.mls.insert(String::new());
            }
            DomainEvent::MlsSendOutcome { group_id, local_id, post_id, state } => {
                self.mls_send_outcome(group_id, local_id, post_id.as_deref(), *state, changes);
            }
            DomainEvent::MlsDeleteOutcome { group_id, post_id, result } => {
                self.mls_delete_outcome(group_id, post_id, result, changes);
            }
            DomainEvent::MlsLeft { group_id, result } => self.mls_left(group_id, result, changes),
            DomainEvent::MlsRefreshed { group_id, last_log_seq } => {
                self.mls.refreshed_at.insert(group_id.clone(), at);
                if let Some(tl) = self.mls.timelines.get_mut(group_id)
                    && tl.loaded
                    && !tl.tail_cut
                    && *last_log_seq > tl.last_log_seq
                {
                    tl.gap_pending = true;
                    changes.mls.insert(group_id.clone());
                }
            }
            DomainEvent::MlsMarkedRead { group_id, read_log_seq, unread, unread_mentions } => {
                self.mls_set_read(group_id, *read_log_seq, *unread, *unread_mentions, changes);
            }
            DomainEvent::MlsUnavailable { method, group_id, attempt, local_id } => {
                let call = Unavailable {
                    method,
                    group: group_id.as_deref(),
                    attempt: *attempt,
                    local_id: local_id.as_deref(),
                };
                self.mls_unavailable(&call, changes);
            }
            _ => {}
        }
        Ok(())
    }

    fn mls_started(&mut self, at: Timestamp, start: &MlsStart, changes: &mut Changes) {
        match start {
            MlsStart::Join { attempt } => {
                self.mls.join.last_attempt = self.mls.join.last_attempt.max(*attempt);
                self.mls.join.in_flight = Some(*attempt);
                changes.mls.insert(String::new());
            }
            MlsStart::Send { group_id, local_id, text, reply_to, mentions, retry_of } => {
                if self.mls.read_only.contains_key(group_id) {
                    return;
                }
                self.mls_open_timeline(group_id);
                let tl = self.mls.timelines.entry(group_id.clone()).or_default();
                if let Some(old) = retry_of {
                    tl.own.retain(|i| &i.local_id != old);
                }
                if tl.own.iter().any(|i| &i.local_id == local_id) {
                    return;
                }
                let order = tl.next_order;
                tl.next_order += 1;
                tl.own.push(OwnItem {
                    local_id: local_id.clone(),
                    post_id: None,
                    text: text.clone(),
                    reply_to: reply_to.clone(),
                    mentions: mentions.clone(),
                    send: SendState::Submitting,
                    at,
                    order,
                });
                self.mls_reproject(group_id, changes);
            }
            MlsStart::History { group_id, direction } => {
                if matches!(direction, HistoryDirection::Newest) {
                    self.mls_open_timeline(group_id);
                }
                if let Some(tl) = self.mls.timelines.get_mut(group_id) {
                    match direction {
                        HistoryDirection::After(_) => tl.gap_in_flight = true,
                        HistoryDirection::Before(_) => tl.older_in_flight = true,
                        HistoryDirection::Newest => {}
                    }
                    changes.mls.insert(group_id.clone());
                }
            }
            MlsStart::Refresh { group_id } => {
                self.mls.refreshed_at.insert(group_id.clone(), at);
            }
            MlsStart::Delete { group_id, post_id } => {
                if let Some(tl) = self.mls.timelines.get_mut(group_id) {
                    tl.deleting.insert(post_id.clone());
                }
                self.mls.delete_failure.remove(post_id);
                self.mls_reproject(group_id, changes);
            }
            MlsStart::Leave { group_id } => {
                self.mls.leaving.insert(group_id.clone());
                self.mls.leave_failure.remove(group_id);
                changes.mls.insert(group_id.clone());
            }
            MlsStart::Create => {
                self.mls.create.in_flight = true;
                changes.mls.insert(String::new());
            }
            MlsStart::List => {
                self.mls.list_wanted = false;
                if self.mls.list.phase == ListPhase::Idle {
                    self.mls.list.phase = ListPhase::Loading;
                }
                changes.mls.insert(String::new());
            }
            MlsStart::Info { group_id } => {
                self.mls.info_wanted.remove(group_id);
                self.mls.info_at.insert(group_id.clone(), at);
            }
        }
    }

    fn mls_listed(
        &mut self,
        rows: &[MlsGroupSummary],
        cursor: Option<String>,
        first_page: bool,
        changes: &mut Changes,
    ) {
        if first_page {
            self.mls.stale = self.mls.groups.keys().cloned().collect();
        }
        self.mls.list_wanted = false;
        for row in rows {
            self.mls.stale.remove(&row.group_id);
            let mut row = row.clone();
            // A newer event wins over a list row (events are newer).
            if let Some(seen) = self.mls.activity.get(&row.group_id)
                && row.last_activity_at.is_none_or(|a| a < *seen)
            {
                row.last_activity_at = Some(*seen);
            }
            // A page later in the listing replaces a stored row only when it
            // is not older.
            if let Some(stored) = self.mls.groups.get(&row.group_id)
                && !first_page
                && row.last_activity_at < stored.last_activity_at
            {
                continue;
            }
            self.mls_upsert_group(row, changes);
        }
        self.mls.list.cursor = cursor.clone();
        self.mls.list.phase = ListPhase::Ready;
        self.mls.list.complete = cursor.is_none();
        if cursor.is_none() {
            let stale: Vec<String> = std::mem::take(&mut self.mls.stale).into_iter().collect();
            for group in stale {
                self.mls_drop_from_list(&group, changes);
            }
        }
        changes.mls.insert(String::new());
    }

    /// Removes a group from the list (the context stays while a timeline of
    /// it is kept, so a read-only chat stays readable).
    fn mls_drop_from_list(&mut self, group: &str, changes: &mut Changes) {
        self.mls.groups.remove(group);
        if !self.mls.timelines.contains_key(group) {
            let context = mls::context_id(group);
            if self.contexts.contains_key(&context) {
                let _ = self.remove_context(&context, changes);
            }
        }
        changes.mls.insert(group.to_owned());
        changes.mls.insert(String::new());
    }

    fn mls_upsert_group(&mut self, summary: MlsGroupSummary, changes: &mut Changes) {
        let group = summary.group_id.clone();
        let context = Context {
            last_activity: summary.last_activity_at,
            id: mls::context_id(&group),
            kind: ContextKind::MlsGroup,
            title: summary.name.clone(),
            participants: Vec::new(),
            pinned: false,
            unread: summary.unread,
            muted: false,
            disappearing_secs: None,
            roles: BTreeMap::new(),
            epoch: None,
            preview: None,
        };
        if self.contexts.get(&context.id) != Some(&context) {
            self.upsert_context(&context, changes);
        }
        if let Some(author) = summary.last_post.as_ref().and_then(|p| p.author.as_ref()) {
            let author = author.clone();
            self.mls_learn_author(&group, &author);
        }
        if self.mls.groups.get(&group) != Some(&summary) {
            self.mls.groups.insert(group.clone(), summary);
            changes.mls.insert(group.clone());
        }
    }

    fn mls_failed(
        &mut self,
        method: &str,
        group: Option<&str>,
        _invite: Option<&str>,
        code: &str,
        changes: &mut Changes,
    ) {
        self.mls.failures.insert((method.to_owned(), group.map(str::to_owned)), code.to_owned());
        if method == crate::intent::methods::MLS_GROUP_LIST {
            self.mls.list.phase = ListPhase::Failed(code.to_owned());
        }
        if let (Some(group), crate::intent::methods::MLS_GROUP_HISTORY) = (group, method)
            && let Some(tl) = self.mls.timelines.get_mut(group)
        {
            tl.older_in_flight = false;
            tl.gap_in_flight = false;
            // No automatic retry: the gap stays open until the next need.
            tl.gap_pending = false;
        }
        if method == crate::intent::methods::MLS_GROUP_JOIN_PENDING {
            self.mls.pending_wanted = false;
        }
        changes.mls.insert(group.unwrap_or_default().to_owned());
        changes.mls.insert(String::new());
    }

    fn mls_members_page(
        &mut self,
        group: &str,
        rows: &[MemberRow],
        cursor: Option<String>,
        failed: Option<String>,
        changes: &mut Changes,
    ) {
        for row in rows {
            self.mls_learn_member(group, row);
        }
        let pages = self.mls.members.entry(group.to_owned()).or_default();
        match failed {
            Some(code) => pages.failed = Some(code),
            None => {
                pages.failed = None;
                pages.loaded = true;
                pages.cursor = cursor;
                for row in rows {
                    let mut row = row.clone();
                    if self.mls.ai_true.contains(&(group.to_owned(), row.anchor.clone())) {
                        row.ai_flag = true;
                    }
                    pages.rows.insert(row.anchor.clone(), row);
                }
            }
        }
        changes.mls.insert(group.to_owned());
    }

    fn mls_learn_member(&mut self, group: &str, row: &MemberRow) {
        if row.ai_flag {
            self.mls.ai_true.insert((group.to_owned(), row.anchor.clone()));
        }
        if row.is_me {
            self.mls.my_anchor = Some(row.anchor.clone());
        }
    }

    fn mls_learn_author(&mut self, group: &str, author: &Author) {
        if author.ai_flag {
            self.mls.ai_true.insert((group.to_owned(), author.anchor.clone()));
        }
        if author.is_me {
            self.mls.my_anchor = Some(author.anchor.clone());
        }
    }

    fn mls_learn_post(&mut self, group: &str, post: &Post) {
        if let Some(author) = &post.author {
            let author = author.clone();
            self.mls_learn_author(group, &author);
        }
        match &post.body {
            PostBody::Membership { changes, by } => {
                for change in changes {
                    if change.ai_flag {
                        self.mls.ai_true.insert((group.to_owned(), change.anchor.clone()));
                    }
                }
                if let Some(by) = by {
                    let by = by.clone();
                    self.mls_learn_author(group, &by);
                }
            }
            PostBody::Settings { by: Some(by), .. } | PostBody::Void { by: Some(by), .. } => {
                let by = by.clone();
                self.mls_learn_author(group, &by);
            }
            _ => {}
        }
    }

    // ---- timelines ------------------------------------------------------

    fn mls_open_timeline(&mut self, group: &str) {
        self.mls.tick += 1;
        let tick = self.mls.tick;
        if !self.mls.timelines.contains_key(group) {
            // The own posts kept while the timeline was gone come back.
            let own = self.mls.parked.remove(group).unwrap_or_default();
            let next_order = own.iter().map(|i| i.order + 1).max().unwrap_or(0);
            let tl = MlsTimeline { own, next_order, ..MlsTimeline::default() };
            self.mls.timelines.insert(group.to_owned(), tl);
        }
        if let Some(tl) = self.mls.timelines.get_mut(group) {
            tl.used = tick;
        }
    }

    /// Keeps the three most recently used timelines and drops the others
    /// with their moments.
    fn mls_trim_timelines(&mut self, keep: &str, changes: &mut Changes) {
        while self.mls.timelines.len() > mls::KEPT_TIMELINES {
            let Some(oldest) = self
                .mls
                .timelines
                .iter()
                .filter(|(g, _)| g.as_str() != keep)
                .min_by_key(|(_, t)| t.used)
                .map(|(g, _)| g.clone())
            else {
                break;
            };
            if let Some(tl) = self.mls.timelines.remove(&oldest) {
                // A post still being sent, or not sent, is not evicted with the
                // window: queued ones come back through history, these do not.
                let kept: Vec<OwnItem> = tl
                    .own
                    .into_iter()
                    .filter(|i| matches!(i.send, SendState::Submitting | SendState::NotSent(_)))
                    .collect();
                if !kept.is_empty() {
                    self.mls.parked.insert(oldest.clone(), kept);
                }
            }
            if let Some(ids) = self.mls.projected.remove(&oldest) {
                for id in ids.keys() {
                    self.remove_projected(id, changes);
                }
            }
            changes.mls.insert(oldest.clone());
            // A read-only chat of a group left the list: its context goes too.
            if !self.mls.groups.contains_key(&oldest) {
                let context = mls::context_id(&oldest);
                if self.contexts.contains_key(&context) {
                    let _ = self.remove_context(&context, changes);
                }
            }
        }
    }

    fn mls_history_page(&mut self, page: &HistoryPage<'_>, changes: &mut Changes) {
        let HistoryPage { group, direction, posts, floor, last, more } = *page;
        if !self.mls.groups.contains_key(group)
            && !self.contexts.contains_key(&mls::context_id(group))
        {
            return;
        }
        for post in posts {
            self.mls_learn_post(group, post);
        }
        self.mls_open_timeline(group);
        let known_anchor = self.mls.my_anchor.clone();
        let tl = self.mls.timelines.entry(group.to_owned()).or_default();
        tl.floor_log_seq = floor.or(tl.floor_log_seq);
        match direction {
            HistoryDirection::Newest => {
                tl.posted.clear();
                tl.tail_cut = false;
                tl.loaded = true;
                tl.more_older = more;
                tl.last_log_seq = last;
                tl.gap_pending = false;
                tl.gap_dirty = false;
            }
            HistoryDirection::Before(_) => {
                tl.older_in_flight = false;
                tl.more_older = more;
            }
            HistoryDirection::After(_) => {
                tl.gap_in_flight = false;
                tl.last_log_seq = tl.last_log_seq.max(last);
                if more {
                    tl.gap_pending = true;
                } else {
                    tl.gap_pending = tl.gap_dirty;
                    tl.gap_dirty = false;
                }
            }
        }
        for post in posts {
            merge_post(tl, post.clone(), known_anchor.as_deref());
        }
        trim_window(tl, direction);
        self.mls_trim_timelines(group, changes);
        self.mls_reproject(group, changes);
    }

    fn mls_group_event(
        &mut self,
        at: Timestamp,
        seq: u64,
        group: &str,
        kind: &GroupEventKind,
        changes: &mut Changes,
    ) {
        // Deduplicate by sequence number (an event with a log position is
        // also deduplicated by the timeline merge).
        if !self.mls.seen_seq.insert(seq) {
            return;
        }
        if self.mls.seen_seq.len() > SEEN_SEQ {
            self.mls.seen_seq.pop_first();
        }
        match kind {
            GroupEventKind::Post(post)
            | GroupEventKind::Membership(post)
            | GroupEventKind::Settings(post) => {
                self.mls_post_event(at, group, post, kind, changes);
            }
            GroupEventKind::PostState { post_id, state } => {
                self.mls_post_state(at, group, post_id, *state, changes);
            }
            GroupEventKind::PostDeleted { post_id, post_log_seq, .. } => {
                self.mls_tombstone(group, post_id, *post_log_seq, changes);
            }
            GroupEventKind::JoinResult { request_id, group_id, result } => {
                self.mls_join_result(request_id, group_id, result, changes);
            }
            GroupEventKind::Read { read_log_seq, unread, unread_mentions } => {
                self.mls_set_read(group, *read_log_seq, *unread, *unread_mentions, changes);
            }
        }
    }

    fn mls_post_event(
        &mut self,
        at: Timestamp,
        group: &str,
        post: &Post,
        kind: &GroupEventKind,
        changes: &mut Changes,
    ) {
        self.mls_learn_post(group, post);
        self.mls.activity.insert(group.to_owned(), post.appended_at.unwrap_or(at));
        if let Some(summary) = self.mls.groups.get_mut(group) {
            summary.last_activity_at = Some(post.appended_at.unwrap_or(at));
            if let Some(ls) = post.log_seq {
                summary.last_log_seq = summary.last_log_seq.max(ls);
            }
            changes.mls.insert(group.to_owned());
            changes.mls.insert(String::new());
            if let Some(context) = self.contexts.get_mut(&mls::context_id(group)) {
                context.last_activity = summary_activity(post, at);
                changes.contexts.insert(context.id.clone());
            }
        }
        if matches!(kind, GroupEventKind::Membership(_) | GroupEventKind::Settings(_)) {
            self.mls.info_wanted.insert(group.to_owned());
        }
        if let PostBody::Membership { changes: rows, .. } = &post.body {
            self.mls_membership_effects(group, rows, changes);
        }
        if let Some(tl) = self.mls.timelines.get_mut(group) {
            let my = self.mls.my_anchor.clone();
            if let Some(ls) = post.log_seq {
                if tl.loaded && !tl.tail_cut {
                    if ls > tl.last_log_seq + 1 {
                        if tl.gap_in_flight {
                            tl.gap_dirty = true;
                        } else {
                            tl.gap_pending = true;
                        }
                    } else {
                        tl.last_log_seq = tl.last_log_seq.max(ls);
                    }
                    merge_post(tl, post.clone(), my.as_deref());
                } else {
                    tl.last_log_seq = tl.last_log_seq.max(ls);
                }
            } else {
                merge_post(tl, post.clone(), my.as_deref());
            }
            trim_window(tl, HistoryDirection::After(0));
            self.mls_reproject(group, changes);
        }
    }

    /// What a membership row does to the roster and to this account's own
    /// standing in the group.
    fn mls_membership_effects(
        &mut self,
        group: &str,
        rows: &[MembershipChange],
        changes: &mut Changes,
    ) {
        let my = self.mls.my_anchor.clone();
        for change in rows {
            let me = change.is_me || my.as_deref().is_some_and(|a| a == change.anchor);
            if let Some(pages) = self.mls.members.get_mut(group) {
                match change.kind {
                    MembershipKind::Left | MembershipKind::Removed | MembershipKind::Banned => {
                        pages.rows.remove(&change.anchor);
                    }
                    MembershipKind::RoleChanged => {
                        if let (Some(row), Some(role)) =
                            (pages.rows.get_mut(&change.anchor), change.role.clone())
                        {
                            row.role = role;
                        }
                    }
                    _ => {}
                }
                changes.mls.insert(group.to_owned());
            }
            if !me {
                continue;
            }
            let reason = match change.kind {
                MembershipKind::Left => Some(ReadOnly::Left),
                MembershipKind::Removed => Some(ReadOnly::Removed),
                MembershipKind::Banned => Some(ReadOnly::Banned),
                _ => None,
            };
            if let Some(reason) = reason {
                self.mls.read_only.insert(group.to_owned(), reason);
                self.mls.groups.remove(group);
                self.mls.info_wanted.remove(group);
                changes.mls.insert(group.to_owned());
                changes.mls.insert(String::new());
            }
        }
    }

    fn mls_post_state(
        &mut self,
        at: Timestamp,
        group: &str,
        post_id: &str,
        state: SendState,
        changes: &mut Changes,
    ) {
        let Some(tl) = self.mls.timelines.get_mut(group) else {
            if let Some(own) = self.mls.parked.get_mut(group) {
                match state {
                    SendState::Sent { .. } => own.retain(|i| i.post_id.as_deref() != Some(post_id)),
                    other => {
                        for item in own.iter_mut().filter(|i| i.post_id.as_deref() == Some(post_id))
                        {
                            item.send = other;
                        }
                    }
                }
            }
            return;
        };
        let Some(ix) = tl.own.iter().position(|i| i.post_id.as_deref() == Some(post_id)) else {
            return;
        };
        match state {
            SendState::Sent { log_seq } => {
                let item = tl.own.remove(ix);
                // Any other row of the same post goes with it.
                tl.own.retain(|i| i.post_id.as_deref() != Some(post_id));
                let my = self.mls.my_anchor.clone();
                let post = own_as_post(&item, Some(log_seq), my.as_deref());
                merge_post(tl, post, my.as_deref());
                tl.last_log_seq = tl.last_log_seq.max(log_seq);
            }
            other => {
                for item in tl.own.iter_mut().filter(|i| i.post_id.as_deref() == Some(post_id)) {
                    item.send = other;
                }
            }
        }
        let _ = at;
        self.mls_reproject(group, changes);
    }

    fn mls_tombstone(
        &mut self,
        group: &str,
        post_id: &str,
        post_log_seq: Option<u64>,
        changes: &mut Changes,
    ) {
        let Some(tl) = self.mls.timelines.get_mut(group) else { return };
        tl.deleting.remove(post_id);
        let key = post_log_seq
            .filter(|ls| tl.posted.contains_key(ls))
            .or_else(|| tl.posted.iter().find(|(_, p)| p.post_id == post_id).map(|(ls, _)| *ls));
        if let Some(ls) = key
            && let Some(post) = tl.posted.get_mut(&ls)
        {
            post.state = PostState::Deleted;
            post.body = PostBody::Opaque;
        }
        tl.own.retain(|i| i.post_id.as_deref() != Some(post_id));
        self.mls_reproject(group, changes);
    }

    fn mls_delete_outcome(
        &mut self,
        group: &str,
        post_id: &str,
        result: &DeleteResult,
        changes: &mut Changes,
    ) {
        match result {
            DeleteResult::Started => {
                if let Some(tl) = self.mls.timelines.get_mut(group) {
                    tl.deleting.insert(post_id.to_owned());
                }
                self.mls_reproject(group, changes);
            }
            DeleteResult::Committed { .. } | DeleteResult::Queued | DeleteResult::Delayed => {
                // The local row turns deleted at once.
                self.mls_tombstone(group, post_id, None, changes);
            }
            DeleteResult::Failed { code } => {
                self.mls.delete_failure.insert(post_id.to_owned(), code.clone());
                if let Some(tl) = self.mls.timelines.get_mut(group) {
                    tl.deleting.remove(post_id);
                }
                self.mls_reproject(group, changes);
            }
        }
        changes.mls.insert(group.to_owned());
    }

    fn mls_left(&mut self, group: &str, result: &LeaveResult, changes: &mut Changes) {
        self.mls.leaving.remove(group);
        match result {
            LeaveResult::Committed => {
                self.mls.leave_failure.remove(group);
                self.mls.read_only.insert(group.to_owned(), ReadOnly::Left);
            }
            other => {
                self.mls.leave_failure.insert(group.to_owned(), other.clone());
            }
        }
        changes.mls.insert(group.to_owned());
        changes.mls.insert(String::new());
    }

    fn mls_set_read(
        &mut self,
        group: &str,
        read: u64,
        unread: u32,
        mentions: u32,
        changes: &mut Changes,
    ) {
        if let Some(summary) = self.mls.groups.get_mut(group) {
            // The position only moves forward.
            if read >= summary.read_log_seq {
                summary.read_log_seq = read;
                summary.unread = unread;
                summary.unread_mentions = mentions;
                if let Some(context) = self.contexts.get_mut(&mls::context_id(group)) {
                    context.unread = unread;
                    changes.contexts.insert(context.id.clone());
                }
                changes.mls.insert(group.to_owned());
                changes.mls.insert(String::new());
            }
        }
    }

    fn mls_unavailable(&mut self, call: &Unavailable<'_>, changes: &mut Changes) {
        let Unavailable { method, group, attempt, local_id } = *call;
        self.mls.unavailable = Some(method.to_owned());
        if let Some(attempt) = attempt {
            self.mls_join_outcome(attempt, &JoinOutcome::Unavailable, changes);
        }
        if let (Some(group), Some(local_id)) = (group, local_id) {
            self.mls_send_outcome(
                group,
                local_id,
                None,
                SendState::NotSent(NotSentReason::Unknown),
                changes,
            );
        }
        match method {
            crate::intent::methods::MLS_GROUP_LIST => {
                self.mls.list.phase = ListPhase::Failed("unavailable".to_owned());
            }
            crate::intent::methods::MLS_GROUP_CREATE => {
                self.mls.create.in_flight = false;
                self.mls.create.seq += 1;
                self.mls.create.outcome = Some(Err("unavailable".to_owned()));
            }
            _ => {
                self.mls.failures.insert(
                    (method.to_owned(), group.map(str::to_owned)),
                    "unavailable".to_owned(),
                );
            }
        }
        changes.mls.insert(String::new());
        if let Some(group) = group {
            changes.mls.insert(group.to_owned());
        }
    }

    // ---- join -----------------------------------------------------------

    fn mls_join_outcome(&mut self, attempt: u64, outcome: &JoinOutcome, changes: &mut Changes) {
        let join = &mut self.mls.join;
        join.last_attempt = join.last_attempt.max(attempt);
        if join.in_flight.is_some_and(|a| a <= attempt) {
            join.in_flight = None;
        }
        match outcome {
            JoinOutcome::Pending { request_id, reason, expires_at, group_id } => {
                join.records.insert(
                    request_id.clone(),
                    JoinRecord {
                        request_id: request_id.clone(),
                        reason: reason.clone(),
                        expires_at: *expires_at,
                        group_id: group_id.clone(),
                        attempt,
                    },
                );
            }
            JoinOutcome::Joined { group_id } => {
                join.records.retain(|_, r| r.group_id.as_deref() != Some(group_id));
                self.mls.list_wanted = true;
            }
            JoinOutcome::Refused(JoinRefusal::AlreadyMember { .. }) => {
                self.mls.list_wanted = true;
            }
            _ => {}
        }
        let join = &mut self.mls.join;
        // A late outcome of an older attempt never replaces a newer one.
        if join.last.as_ref().is_none_or(|(a, _)| attempt >= *a) {
            join.last = Some((attempt, outcome.clone()));
        }
        changes.mls.insert(String::new());
    }

    fn mls_join_result(
        &mut self,
        request_id: &str,
        group_id: &str,
        result: &Result<(), JoinRefusal>,
        changes: &mut Changes,
    ) {
        // Applied to the pending request with this id only. An unknown id
        // changes no record; it only asks for a list read.
        self.mls.list_wanted = true;
        let Some(record) = self.mls.join.records.remove(request_id) else {
            changes.mls.insert(String::new());
            return;
        };
        if record.attempt > 0
            && self.mls.join.last.as_ref().is_none_or(|(a, _)| record.attempt >= *a)
        {
            let outcome = match result {
                Ok(()) => JoinOutcome::Joined { group_id: group_id.to_owned() },
                Err(refusal) => JoinOutcome::Refused(refusal.clone()),
            };
            self.mls.join.last = Some((record.attempt, outcome));
        }
        changes.mls.insert(String::new());
    }

    // ---- send -----------------------------------------------------------

    fn mls_send_outcome(
        &mut self,
        group: &str,
        local_id: &str,
        post_id: Option<&str>,
        state: SendState,
        changes: &mut Changes,
    ) {
        let my = self.mls.my_anchor.clone();
        // A daily limit used up locks the composer until it resets.
        if let SendState::NotSent(NotSentReason::RateLimited {
            resets_at: Some(at),
            remaining: 0,
            ..
        }) = state
        {
            self.mls.rate_locks.insert(group.to_owned(), at);
            changes.mls.insert(group.to_owned());
        }
        // The outcome of a send lands even when the timeline was evicted.
        let Some(tl) = self.mls.timelines.get_mut(group) else {
            if let Some(own) = self.mls.parked.get_mut(group)
                && let Some(ix) = own.iter().position(|i| i.local_id == local_id)
            {
                match state {
                    // It comes back through history.
                    SendState::Sent { .. } => {
                        own.remove(ix);
                    }
                    other => {
                        own[ix].post_id = post_id.map(str::to_owned).or(own[ix].post_id.take());
                        own[ix].send = other;
                    }
                }
            }
            return;
        };
        let Some(ix) = tl.own.iter().position(|i| i.local_id == local_id) else {
            // Merged already by an event, or cleared: nothing to apply.
            return;
        };
        if let Some(post_id) = post_id {
            tl.own[ix].post_id = Some(post_id.to_owned());
            // The SDK's own unsent row may have arrived while the call ran:
            // one post, one bubble (dedupe by `post_id`).
            tl.own.retain(|i| i.local_id == local_id || i.post_id.as_deref() != Some(post_id));
        }
        let Some(ix) = tl.own.iter().position(|i| i.local_id == local_id) else { return };
        match state {
            SendState::Sent { log_seq } => {
                let item = tl.own.remove(ix);
                let post = own_as_post(&item, Some(log_seq), my.as_deref());
                merge_post(tl, post, my.as_deref());
                tl.last_log_seq = tl.last_log_seq.max(log_seq);
            }
            other => tl.own[ix].send = other,
        }
        self.mls_reproject(group, changes);
    }

    // ---- projection -----------------------------------------------------

    /// Brings the moments of a group's context in line with its timeline.
    pub(super) fn mls_reproject(&mut self, group: &str, changes: &mut Changes) {
        changes.mls.insert(group.to_owned());
        let context = mls::context_id(group);
        if !self.contexts.contains_key(&context) {
            // A read-only chat whose context was dropped has nothing to show.
            return;
        }
        let Some(tl) = self.mls.timelines.get(group) else { return };
        let me = crate::selectors::self_participant(self)
            .unwrap_or_else(|| ParticipantId::new(format!("{}me", mls::PARTICIPANT_PREFIX)));
        let wanted = self.mls_project(group, tl, &context, &me);
        // Participants of the authors.
        let mut people: BTreeMap<ParticipantId, (String, bool)> = BTreeMap::new();
        for post in tl.posted.values() {
            if let Some(author) = actor(post).filter(|_| post.author.is_some()) {
                if author.is_me {
                    continue;
                }
                let id = participant_of(group, &author.anchor);
                let ai = author.ai_flag
                    || self.mls.ai_true.contains(&(group.to_owned(), author.anchor.clone()));
                people.insert(id, (author.display.clone().unwrap_or_default(), ai));
            }
        }
        for (id, (display, ai)) in people {
            let next = mls_participant(id.clone(), &display, ai);
            if self.participants.get(&id) != Some(&next) {
                self.participants.insert(id.clone(), next);
                changes.participants.insert(id);
            }
        }
        let had = self.mls.projected.remove(group).unwrap_or_default();
        let mut now: BTreeMap<MomentId, Moment> = BTreeMap::new();
        for id in had.keys() {
            if !wanted.iter().any(|m| &m.id == id) {
                self.remove_projected(id, changes);
            }
        }
        let at = self.mls.clock;
        for moment in wanted {
            match had.get(&moment.id) {
                Some(old) if old == &moment => {}
                Some(old)
                    if std::mem::discriminant(&old.kind)
                        == std::mem::discriminant(&moment.kind)
                        && old.lifecycle == moment.lifecycle
                        && old.order == moment.order
                        && self.moments.contains_key(&moment.id) =>
                {
                    let id = moment.id.clone();
                    let kind = moment.kind.clone();
                    let _ = self.update_moment(at, &id, None, Some(&kind), changes);
                    if let Some(stored) = self.moments.get_mut(&id) {
                        stored.at = moment.at;
                    }
                }
                Some(_) => {
                    self.remove_projected(&moment.id, changes);
                    let _ = self.add_moment(at, &moment, changes);
                }
                None => {
                    // A moment that exists under this id (a stale one) is replaced.
                    self.remove_projected(&moment.id, changes);
                    let _ = self.add_moment(at, &moment, changes);
                }
            }
            now.insert(moment.id.clone(), moment);
        }
        self.mls.projected.insert(group.to_owned(), now);
    }

    fn mls_project(
        &self,
        group: &str,
        tl: &MlsTimeline,
        context: &ContextId,
        me: &ParticipantId,
    ) -> Vec<Moment> {
        let my = self.mls.my_anchor.as_deref();
        let mut out: Vec<Moment> = Vec::new();
        let mut last_at = self.mls.clock;
        let id_of = |ls: u64| MomentId::new(format!("mls:{group}:{ls}"));
        let by_post: BTreeMap<&str, MomentId> = tl
            .posted
            .iter()
            .map(|(ls, p)| (p.post_id.as_str(), id_of(*ls)))
            .chain(tl.own.iter().filter_map(|i| {
                i.post_id.as_deref().map(|p| (p, MomentId::new(format!("mls:{group}:p:{p}"))))
            }))
            .collect();
        let reply = |target: &Option<String>| {
            target
                .as_deref()
                .and_then(|t| by_post.get(t))
                .map(|id| ReplyRef { message_id: id.clone() })
        };
        let make = |id: MomentId, origin: ParticipantId, at: Timestamp, kind, lifecycle, order| {
            Moment { id, context_id: context.clone(), origin, at, kind, lifecycle, order }
        };
        if tl.floor_log_seq.is_some() && !tl.more_older && tl.loaded {
            out.push(make(
                MomentId::new(format!("mls:{group}:floor")),
                service_participant(),
                Timestamp(0),
                MomentKind::MlsService(MlsServiceLine::Floor),
                Lifecycle::Done,
                Some(OrderKey { hlc: 0, author: String::new(), device_sequence: 0 }),
            ));
        }
        let mut lines: Vec<Line> = Vec::new();
        for (ls, post) in &tl.posted {
            let at = post.appended_at.unwrap_or(last_at);
            last_at = at;
            let id = id_of(*ls);
            if post.state == PostState::Deleted {
                lines.push(Line::Single(MlsServiceLine::Deleted, id.as_str().to_owned(), *ls));
                continue;
            }
            match (&post.kind, &post.body) {
                (PostKind::Text, PostBody::Text(text)) => {
                    let (origin, mine) = match &post.author {
                        Some(a) if a.is_me || my == Some(a.anchor.as_str()) => (me.clone(), true),
                        Some(a) => (participant_of(group, &a.anchor), false),
                        None => (service_participant(), false),
                    };
                    let _ = mine;
                    out.push(make(
                        id,
                        origin,
                        at,
                        MomentKind::Message(MessagePayload {
                            text: text.clone(),
                            receipt: Receipt::Sent,
                            reply_to: reply(&post.reply_to),
                            ..Default::default()
                        }),
                        Lifecycle::Done,
                        Some(key_posted(*ls)),
                    ));
                }
                (PostKind::Attachment | PostKind::Card, _) => {
                    let origin = match &post.author {
                        Some(a) if a.is_me || my == Some(a.anchor.as_str()) => me.clone(),
                        Some(a) => participant_of(group, &a.anchor),
                        None => service_participant(),
                    };
                    out.push(make(
                        id,
                        origin,
                        at,
                        MomentKind::Fallback(FallbackPayload {
                            reason: "mls.opaque".to_owned(),
                            text: String::new(),
                            unsupported: None,
                        }),
                        Lifecycle::Done,
                        Some(key_posted(*ls)),
                    ));
                }
                (_, PostBody::Membership { changes: rows, by }) => {
                    Self::mls_membership_lines(&mut lines, *ls, id.as_str(), rows, by.as_ref(), my);
                }
                (_, PostBody::Settings { change, by }) => {
                    let by_name = by.as_ref().map(name_of);
                    let mut any = false;
                    if let Some(name) = &change.name {
                        any = true;
                        lines.push(Line::Single(
                            MlsServiceLine::Renamed { by: by_name.clone(), name: name.clone() },
                            format!("{}:n", id.as_str()),
                            *ls,
                        ));
                    }
                    if let Some(on) = change.approval {
                        any = true;
                        lines.push(Line::Single(
                            MlsServiceLine::Approval { by: by_name.clone(), on },
                            format!("{}:a", id.as_str()),
                            *ls,
                        ));
                    }
                    if !any {
                        lines.push(Line::Single(
                            MlsServiceLine::Updated,
                            id.as_str().to_owned(),
                            *ls,
                        ));
                    }
                }
                (_, PostBody::Void { .. }) => {
                    lines.push(Line::Single(MlsServiceLine::Void, id.as_str().to_owned(), *ls));
                }
                _ => {
                    lines.push(Line::Single(MlsServiceLine::Updated, id.as_str().to_owned(), *ls));
                }
            }
        }
        let at_for_line =
            |ls: u64| tl.posted.get(&ls).and_then(|p| p.appended_at).unwrap_or(self.mls.clock);
        for line in lines {
            let (service, id, ls) = match line {
                Line::Joined(who, ls) => {
                    (MlsServiceLine::Joined { who }, format!("mls:{group}:{ls}"), ls)
                }
                Line::Left(who, ls) => {
                    (MlsServiceLine::Left { who }, format!("mls:{group}:{ls}"), ls)
                }
                Line::Single(service, id, ls) => (service, id, ls),
            };
            out.push(make(
                MomentId::new(id),
                service_participant(),
                at_for_line(ls),
                MomentKind::MlsService(service),
                Lifecycle::Done,
                Some(key_posted(ls)),
            ));
        }
        for item in &tl.own {
            let (receipt, lifecycle) = match item.send {
                SendState::Submitting => (Receipt::Sending, Lifecycle::Pending),
                SendState::Sent { .. } => (Receipt::Sent, Lifecycle::Done),
                SendState::Queued(reason) => (Receipt::Queued { reason }, Lifecycle::Pending),
                SendState::Delayed { send_after } => {
                    (Receipt::Delayed { send_after }, Lifecycle::Pending)
                }
                SendState::NotSent(reason) => (Receipt::NotSent { reason }, Lifecycle::Failed),
            };
            let id = match &item.post_id {
                Some(p) => format!("mls:{group}:p:{p}"),
                None => format!("mls:{group}:l:{}", item.local_id),
            };
            out.push(make(
                MomentId::new(id),
                me.clone(),
                item.at,
                MomentKind::Message(MessagePayload {
                    text: item.text.clone(),
                    receipt,
                    reply_to: reply(&item.reply_to),
                    ..Default::default()
                }),
                lifecycle,
                Some(key_own(item.order)),
            ));
        }
        out
    }

    fn mls_membership_lines(
        lines: &mut Vec<Line>,
        ls: u64,
        id: &str,
        rows: &[MembershipChange],
        by: Option<&Author>,
        my: Option<&str>,
    ) {
        let shown: Vec<&MembershipChange> = rows
            .iter()
            .filter(|c| {
                !matches!(c.kind, MembershipKind::DeviceAdded | MembershipKind::DeviceRemoved)
            })
            .collect();
        if shown.is_empty() {
            return;
        }
        let by_name = by.map(name_of);
        let joined: Vec<MlsName> = shown
            .iter()
            .filter(|c| c.kind == MembershipKind::Joined)
            .map(|c| change_name(c, my))
            .collect();
        let left: Vec<MlsName> = shown
            .iter()
            .filter(|c| c.kind == MembershipKind::Left)
            .map(|c| change_name(c, my))
            .collect();
        let only_joined = joined.len() == shown.len();
        let only_left = left.len() == shown.len();
        if only_joined {
            if let Some(Line::Joined(names, _)) = lines.last_mut() {
                let room = mls::MAX_NAMES.saturating_sub(names.len());
                names.extend(joined.into_iter().take(room));
            } else {
                lines.push(Line::Joined(joined.into_iter().take(mls::MAX_NAMES).collect(), ls));
            }
            return;
        }
        if only_left {
            if let Some(Line::Left(names, _)) = lines.last_mut() {
                let room = mls::MAX_NAMES.saturating_sub(names.len());
                names.extend(left.into_iter().take(room));
            } else {
                lines.push(Line::Left(left.into_iter().take(mls::MAX_NAMES).collect(), ls));
            }
            return;
        }
        if !joined.is_empty() {
            lines.push(Line::Single(
                MlsServiceLine::Joined { who: joined.into_iter().take(mls::MAX_NAMES).collect() },
                format!("{id}:j"),
                ls,
            ));
        }
        if !left.is_empty() {
            lines.push(Line::Single(
                MlsServiceLine::Left { who: left.into_iter().take(mls::MAX_NAMES).collect() },
                format!("{id}:l"),
                ls,
            ));
        }
        for (ix, change) in shown.iter().enumerate() {
            let target = change_name(change, my);
            let service = match &change.kind {
                MembershipKind::Removed => MlsServiceLine::Removed { by: by_name.clone(), target },
                MembershipKind::Banned => MlsServiceLine::Banned { by: by_name.clone(), target },
                MembershipKind::RoleChanged => match &change.role {
                    Some(MlsRole::Admin) => {
                        MlsServiceLine::RoleChanged { by: by_name.clone(), target, admin: true }
                    }
                    Some(MlsRole::Member) => {
                        MlsServiceLine::RoleChanged { by: by_name.clone(), target, admin: false }
                    }
                    _ => MlsServiceLine::Updated,
                },
                MembershipKind::Unknown(_) => MlsServiceLine::Updated,
                _ => continue,
            };
            lines.push(Line::Single(service, format!("{id}:{ix}"), ls));
        }
    }
}

fn summary_activity(post: &Post, at: Timestamp) -> Option<Timestamp> {
    Some(post.appended_at.unwrap_or(at))
}

fn placeholder_summary(group: &str, name: &str, approval: bool, at: Timestamp) -> MlsGroupSummary {
    MlsGroupSummary {
        group_id: group.to_owned(),
        name: name.to_owned(),
        tier: 1000,
        members: 1,
        leaves: 0,
        agents: 0,
        approval,
        my_role: MlsRole::Owner,
        host_node: String::new(),
        push: mls::PushSetting::MentionsReplies,
        catch_up: None,
        privacy_delay: mls::PrivacyDelay::None,
        quota_post: None,
        quota_commit: None,
        last_log_seq: 0,
        warnings: Vec::new(),
        last_activity_at: Some(at),
        unread: 0,
        unread_mentions: 0,
        read_log_seq: 0,
        last_post: None,
    }
}

/// Builds the post an own item becomes once it has a log position.
fn own_as_post(item: &OwnItem, log_seq: Option<u64>, my: Option<&str>) -> Post {
    Post {
        log_seq,
        post_id: item.post_id.clone().unwrap_or_else(|| item.local_id.clone()),
        author: Some(Author {
            anchor: my.unwrap_or_default().to_owned(),
            display: None,
            ai_flag: false,
            is_me: true,
        }),
        appended_at: Some(item.at),
        kind: PostKind::Text,
        body: PostBody::Text(item.text.clone()),
        reply_to: item.reply_to.clone(),
        mentions: item.mentions.clone(),
        state: PostState::Sent,
        send: None,
    }
}

/// Merges a post into a timeline: by log position when it has one (a
/// deleted slot stays deleted), else as an own post by `post_id`.
fn merge_post(tl: &mut MlsTimeline, post: Post, my: Option<&str>) {
    if let Some(ls) = post.log_seq {
        // An own item for this post (by id, or the single submitting item
        // with the same text) is the same post.
        let is_mine =
            post.author.as_ref().is_some_and(|a| a.is_me || my == Some(a.anchor.as_str()));
        if let Some(ix) =
            tl.own.iter().position(|i| i.post_id.as_deref() == Some(post.post_id.as_str()))
        {
            tl.own.remove(ix);
        } else if is_mine && let PostBody::Text(text) = &post.body {
            let candidates: Vec<usize> = tl
                .own
                .iter()
                .enumerate()
                .filter(|(_, i)| {
                    i.post_id.is_none() && i.send == SendState::Submitting && &i.text == text
                })
                .map(|(ix, _)| ix)
                .collect();
            if candidates.len() == 1 {
                tl.own.remove(candidates[0]);
            }
        }
        match tl.posted.get_mut(&ls) {
            Some(existing) if existing.state == PostState::Deleted => {}
            Some(existing) => {
                let keep_deleted = post.state == PostState::Deleted;
                *existing = post;
                if keep_deleted {
                    existing.body = PostBody::Opaque;
                }
            }
            None => {
                let mut post = post;
                if post.state == PostState::Deleted {
                    post.body = PostBody::Opaque;
                }
                tl.posted.insert(ls, post);
            }
        }
        return;
    }
    // An own post that is not in the log (history: `state: pending`).
    let send = post.send.unwrap_or(SendState::Queued(QueueReason::Unknown));
    if let Some(item) =
        tl.own.iter_mut().find(|i| i.post_id.as_deref() == Some(post.post_id.as_str()))
    {
        item.send = send;
        return;
    }
    if tl.posted.values().any(|p| p.post_id == post.post_id) {
        return;
    }
    let PostBody::Text(text) = &post.body else { return };
    let order = tl.next_order;
    tl.next_order += 1;
    tl.own.push(OwnItem {
        local_id: format!("p-{}", post.post_id),
        post_id: Some(post.post_id.clone()),
        text: text.clone(),
        reply_to: post.reply_to.clone(),
        mentions: post.mentions.clone(),
        send,
        at: post.appended_at.unwrap_or_default(),
        order,
    });
}

/// Drops rows from the end that was not scrolled to, keeping the window.
fn trim_window(tl: &mut MlsTimeline, direction: HistoryDirection) {
    while tl.posted.len() > mls::WINDOW {
        match direction {
            HistoryDirection::Before(_) => {
                // Scrolling up: the newest rows go.
                if tl.posted.pop_last().is_some() {
                    tl.tail_cut = true;
                }
            }
            _ => {
                if tl.posted.pop_first().is_some() {
                    tl.more_older = true;
                }
            }
        }
    }
}
