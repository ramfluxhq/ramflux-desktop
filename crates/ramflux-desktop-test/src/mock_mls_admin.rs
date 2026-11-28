// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The admin and edge-state answers of the demo (mock) MLS groups (MLS GUI
//! batch M3, fixtures F3-1 to F3-13), active only while the scenario
//! `s12_mls_groups` is loaded. They extend [`MockMls`](crate::mock_mls::MockMls):
//! `MockMls::respond` hands every `mls_group.*` method it does not know to
//! [`MockMls::respond_admin`], and the first refresh of a group asks
//! [`MockMls::first_refresh`] for its timed events.
//!
//! Every answer carries the ids of the call (`epoch`, `action_id`, the
//! target), so a late answer never lands on another target. Variants that
//! need a different reply are bound to their own target, so a runner needs no
//! switch:
//!
//! - Design: the event-first role change is Ben; Remove Dana answers after
//!   3 s (F3-5); Approve Eve answers after 3 s; Approve Ben answers
//!   `request_not_found`; Approve Dana answers `paced` for 5 s; Decline
//!   answers `declined` after 1 s.
//! - Book club: the stale role is Ana (Remove and Ban answer
//!   `not_admin{required: admin}` and the next group read says `member`);
//!   Remove Chen answers after 3 s and a `membership removed` for this
//!   account arrives 1 s after the call (F3-11); Skip answers after 3 s.
//! - Late catch-up (`g-latecatch`): the skip answers after 3 s, and a
//!   `catch_up idle` event arrives 1 s after the call.
//! - Catch-up zero (`g-catch0`): the total is 0.
//! - Stale head (`g-stale`): a void always answers `void_not_head` with head
//!   252, together with the `membership` row at 252.
//! - Neighbours: the push mode "None" fails with a generic error.
//!
//! Timed events go through delays of the first refresh (opening a group asks
//! for one): Design brings the warnings of F3-8 in turn, Book club the
//! catch-up progress of F3-7, Neighbours the `head_commit_rejected` a member
//! never sees. The F2-7 news of Design and Book club come with the second
//! refresh.

use std::collections::{BTreeMap, BTreeSet};

use ramflux_desktop_core::event::admin_of;
use ramflux_desktop_core::mls_group::{
    AdminResult, CatchUp, CatchUpState, DeclineResult, GroupEventKind, JoinRequest,
    MembershipChange, MembershipKind, MlsGroupSummary, MlsRole, OpaqueId, Post, PostBody, PostKind,
    PostState, PushResult, PushSetting, SettingsChange, SkipResult, Warning, WarningCode,
};
use ramflux_desktop_core::{DomainEvent, JsonValue, Timestamp, methods};
use serde_json::json;

use crate::mock_mls::{Answer, MockMls, author, member_at};

/// The admin and edge state of the demo groups.
#[derive(Debug, Default)]
pub struct AdminState {
    /// Posts sent per group (the post quota counts them after a group read).
    pub(crate) sent: BTreeMap<String, u64>,
    /// Admin commits per group (the commit quota counts them).
    commits: BTreeMap<String, u64>,
    /// Anchors removed or banned.
    pub(crate) gone: BTreeSet<String>,
    /// Roles changed.
    pub(crate) roles: BTreeMap<String, MlsRole>,
    /// Join requests approved or declined.
    done: BTreeSet<String>,
    /// Groups whose requests were listed once.
    listed: BTreeSet<String>,
    /// When Dana's request expires (30 s after the first listing).
    dana_expires: Option<i64>,
    /// Whether page 2 of the queue failed once already.
    queue_page_failed: bool,
    names: BTreeMap<String, String>,
    approvals: BTreeMap<String, bool>,
    push: BTreeMap<String, PushSetting>,
    /// The role this account has now where the fixture changed it.
    my_role: BTreeMap<String, MlsRole>,
    /// Per group: when the catch-up ends, and whether it was skipped.
    catch: BTreeMap<String, (i64, bool)>,
    /// The window in which `fork_alarm` is active: group, from, until.
    fork: Option<(String, i64, i64)>,
}

impl AdminState {
    /// Applies what the admin calls changed to a group as `group.info` and
    /// `group.list` describe it.
    pub(crate) fn decorate(&self, summary: &mut MlsGroupSummary, now: Timestamp) {
        let group = summary.group_id.clone();
        if let Some(name) = self.names.get(&group) {
            summary.name.clone_from(name);
        }
        if let Some(approval) = self.approvals.get(&group) {
            summary.approval = *approval;
        }
        if let Some(push) = self.push.get(&group) {
            summary.push = push.clone();
        }
        if let Some(role) = self.my_role.get(&group) {
            summary.my_role = role.clone();
        }
        // The quotas count what was used since the start; they change only
        // when the next group read arrives (F3-9).
        let sent = self.sent.get(&group).copied().unwrap_or(0);
        let commits = self.commits.get(&group).copied().unwrap_or(0);
        if let Some(quota) = &mut summary.quota_post {
            quota.remaining = quota.remaining.saturating_sub(sent);
        }
        if let Some(quota) = &mut summary.quota_commit {
            quota.remaining = quota.remaining.saturating_sub(commits);
        }
        if let Some((idle_at, skipped)) = self.catch.get(&group) {
            if now.0 >= *idle_at {
                summary.catch_up = None;
            } else if *skipped {
                summary.catch_up =
                    Some(CatchUp { state: "rejoining".to_owned(), done: 0, total: 0 });
            }
        }
        // Admission needs a member online exactly while approval is off.
        summary.warnings.retain(|w| w.code != "admission_needs_online");
        if !summary.approval {
            summary.warnings.push(Warning {
                code: "admission_needs_online".to_owned(),
                params: json!({}),
                cleared: false,
            });
        }
        if let Some((g, from, until)) = &self.fork
            && *g == group
            && (*from..*until).contains(&now.0)
        {
            summary.warnings.push(Warning {
                code: "fork_alarm".to_owned(),
                params: json!({ "head_n": 2 }),
                cleared: false,
            });
        }
    }

    /// How long a `group.info` answer takes: a read issued while `fork_alarm`
    /// is active is answered after it is cleared (the stale read of F3-8).
    pub(crate) fn info_delay(&self, group: &str, now: Timestamp) -> u64 {
        match &self.fork {
            Some((g, from, until)) if g == group && (*from..*until).contains(&now.0) => 5_000,
            _ => 80,
        }
    }
}

/// The request table of Design (F3-1): id, name, AI flag.
const DESIGN_REQUESTS: [(&str, Option<&str>, bool); 7] = [
    ("rq-ana", Some("Ana"), false),
    ("rq-ben", Some("Ben"), false),
    ("rq-chen", Some("Chen"), false),
    ("rq-dana", Some("Dana"), false),
    ("rq-eve", Some("Eve"), false),
    ("rq-scout", Some("Scout"), true),
    ("rq-anon", None, false),
];

/// Requests of the queue (F3-2).
const QUEUE_REQUESTS: u32 = 230;
/// Requests of the first page of the queue.
const QUEUE_PAGE: u32 = 200;

/// The name and AI flag of a request, by id.
fn request_person(id: &str) -> (Option<String>, bool) {
    if let Some((_, name, ai)) = DESIGN_REQUESTS.iter().find(|(rid, _, _)| *rid == id) {
        return (name.map(str::to_owned), *ai);
    }
    if id == "rq-fay" {
        return (Some("Fay".to_owned()), false);
    }
    match id.strip_prefix("rq-q-") {
        Some(n) => (Some(format!("Person {n}")), false),
        None => (None, false),
    }
}

fn request(id: &str, requested: i64, expires: i64) -> JoinRequest {
    let (display, ai_flag) = request_person(id);
    JoinRequest {
        request_id: OpaqueId::new(id),
        anchor: OpaqueId::new(format!("anc-{id}")),
        display,
        ai_flag,
        requested_at: Some(Timestamp(requested)),
        expires_at: Some(Timestamp(expires)),
    }
}

/// The member a roster anchor stands for (`anc-<group>-<NNNN>`).
fn roster_person(anchor: &str) -> (Option<String>, bool) {
    let Some((prefix, n)) = anchor.rsplit_once('-') else { return (None, false) };
    let (Some(group), Ok(i)) = (prefix.strip_prefix("anc-"), n.parse::<u32>()) else {
        return (None, false);
    };
    let row = member_at(group, i);
    (row.display, row.ai_flag)
}

impl MockMls {
    /// The events of a commit row: the post that carries it.
    fn commit_post(
        &mut self,
        group: &str,
        kind: PostKind,
        body: PostBody,
        now: Timestamp,
    ) -> (u64, Post) {
        let log_seq = self.log(group);
        let post = Post {
            log_seq: Some(log_seq),
            post_id: format!("p-{group}-{log_seq}"),
            author: None,
            appended_at: Some(now),
            kind,
            body,
            reply_to: None,
            mentions: Vec::new(),
            state: PostState::Sent,
            send: None,
        };
        (log_seq, post)
    }

    fn me() -> ramflux_desktop_core::mls_group::Author {
        author("anc-me", Some("Me"), false, true)
    }

    fn outcome(&self, method: &str, body: &JsonValue, result: AdminResult) -> Option<DomainEvent> {
        let (kind, target) = admin_of(method, body)?;
        Some(DomainEvent::MlsAdminOutcome {
            epoch: body.get("epoch").and_then(JsonValue::as_u64).unwrap_or(0),
            action_id: body.get("action_id").and_then(JsonValue::as_u64).unwrap_or(0),
            group_id: body
                .get("group_id")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_owned(),
            kind,
            target,
            result,
        })
    }

    fn refusal(code: &str) -> AdminResult {
        AdminResult::Refused { code: code.to_owned(), retry_at: None, head_log_seq: None }
    }

    /// The answers to the M3 methods.
    pub(crate) fn respond_admin(
        &mut self,
        method: &str,
        body: &JsonValue,
        group: &str,
        now: Timestamp,
    ) -> Vec<Answer> {
        match method {
            methods::MLS_GROUP_REQUESTS_LIST => self.requests_list(group, body, now),
            methods::MLS_GROUP_REQUESTS_APPROVE => self.approve(method, group, body, now),
            methods::MLS_GROUP_REQUESTS_DECLINE => {
                let id = body.get("request_id").and_then(JsonValue::as_str).unwrap_or_default();
                self.admin.done.insert(id.to_owned());
                vec![(
                    1_000,
                    DomainEvent::MlsDeclineOutcome {
                        epoch: body.get("epoch").and_then(JsonValue::as_u64).unwrap_or(0),
                        group_id: group.to_owned(),
                        request_id: OpaqueId::new(id),
                        result: DeclineResult::Declined,
                    },
                )]
            }
            methods::MLS_GROUP_ROLE_SET
            | methods::MLS_GROUP_MEMBER_REMOVE
            | methods::MLS_GROUP_MEMBER_BAN => self.member_action(method, group, body, now),
            methods::MLS_GROUP_SETTINGS_SET => self.settings(method, group, body, now),
            methods::MLS_GROUP_PUSH_SET => {
                let push = PushSetting::from_token(
                    body.get("push").and_then(JsonValue::as_str).unwrap_or_default(),
                );
                // F3-6: "None" fails in Neighbours; the old value stays.
                let result = if group == "g-neighbours" && push == PushSetting::None {
                    PushResult::Refused { code: "generic".to_owned() }
                } else {
                    self.admin.push.insert(group.to_owned(), push.clone());
                    PushResult::Set(push)
                };
                vec![(
                    1_000,
                    DomainEvent::MlsPushOutcome {
                        epoch: body.get("epoch").and_then(JsonValue::as_u64).unwrap_or(0),
                        group_id: group.to_owned(),
                        result,
                    },
                )]
            }
            methods::MLS_GROUP_CATCH_UP => self.skip(group, body, now),
            methods::MLS_GROUP_VOID => self.void(method, group, body, now),
            _ => Vec::new(),
        }
    }

    fn requests_list(&mut self, group: &str, body: &JsonValue, now: Timestamp) -> Vec<Answer> {
        let cursor = body.get("cursor").and_then(JsonValue::as_str).map(str::to_owned);
        let page_gen = body.get("page_gen").and_then(JsonValue::as_u64).unwrap_or(0);
        let first = cursor.is_none();
        let page = |rows: Vec<JoinRequest>, next: Option<&str>, failed: Option<&str>| {
            (
                200,
                DomainEvent::MlsRequestsPage {
                    group_id: group.to_owned(),
                    page_gen,
                    first_page: first,
                    rows,
                    cursor: next.map(str::to_owned),
                    failed: failed.map(str::to_owned),
                },
            )
        };
        let day = 86_400_000;
        match group {
            "g-queue" => match cursor.as_deref() {
                None => {
                    let rows = (1..=QUEUE_PAGE)
                        .map(|n| {
                            request(
                                &format!("rq-q-{n:04}"),
                                now.0 - i64::from(QUEUE_REQUESTS - n) * 1_000,
                                now.0 + day,
                            )
                        })
                        .collect();
                    vec![page(rows, Some("r2"), None)]
                }
                Some("r2") if !self.admin.queue_page_failed => {
                    // Page 2 fails once with host_unreachable (F3-2).
                    self.admin.queue_page_failed = true;
                    vec![page(Vec::new(), None, Some("host_unreachable"))]
                }
                Some(_) => {
                    let rows = (QUEUE_PAGE + 1..=QUEUE_REQUESTS)
                        .map(|n| {
                            request(
                                &format!("rq-q-{n:04}"),
                                now.0 - i64::from(QUEUE_REQUESTS - n) * 1_000,
                                now.0 + day,
                            )
                        })
                        .collect();
                    vec![page(rows, None, None)]
                }
            },
            "g-design" => {
                let dana = *self.admin.dana_expires.get_or_insert(now.0 + 30_000);
                let mut out = Vec::new();
                // Fay arrives by event 4 s after the first listing, and the
                // same request arrives once more (F3-1).
                if self.admin.listed.insert(group.to_owned()) {
                    let fay = request("rq-fay", now.0, now.0 + day);
                    for delay in [4_000, 4_600] {
                        let event = self.group_event(
                            group,
                            None,
                            GroupEventKind::PendingJoin(Box::new(fay.clone())),
                        );
                        out.push((delay, event));
                    }
                }
                let rows = DESIGN_REQUESTS
                    .iter()
                    .enumerate()
                    .filter(|(_, (id, _, _))| !self.admin.done.contains(*id))
                    .map(|(ix, (id, _, _))| {
                        let expires = if *id == "rq-dana" { dana } else { now.0 + day };
                        request(id, now.0 - (10 - ix as i64) * 60_000, expires)
                    })
                    .collect();
                out.push(page(rows, None, None));
                out
            }
            _ => vec![page(Vec::new(), None, None)],
        }
    }

    fn approve(
        &mut self,
        method: &str,
        group: &str,
        body: &JsonValue,
        now: Timestamp,
    ) -> Vec<Answer> {
        let id = body.get("request_id").and_then(JsonValue::as_str).unwrap_or_default().to_owned();
        let refused = |this: &Self, delay: u64, result: AdminResult| {
            this.outcome(method, body, result).map(|e| vec![(delay, e)]).unwrap_or_default()
        };
        match id.as_str() {
            "rq-ben" => return refused(self, 300, Self::refusal("request_not_found")),
            "rq-dana" => {
                return refused(
                    self,
                    300,
                    AdminResult::Refused {
                        code: "paced".to_owned(),
                        retry_at: Some(Timestamp(now.0 + 5_000)),
                        head_log_seq: None,
                    },
                );
            }
            _ => {}
        }
        self.admin.done.insert(id.clone());
        *self.admin.commits.entry(group.to_owned()).or_default() += 1;
        let (display, ai_flag) = request_person(&id);
        let slow = id == "rq-eve";
        let (log_seq, post) = self.commit_post(
            group,
            PostKind::Membership,
            PostBody::Membership {
                changes: vec![MembershipChange {
                    kind: MembershipKind::Joined,
                    anchor: format!("anc-{id}"),
                    display,
                    ai_flag,
                    role: None,
                    is_me: false,
                }],
                by: Some(Self::me()),
            },
            now,
        );
        let event =
            self.group_event(group, Some(log_seq), GroupEventKind::Membership(Box::new(post)));
        let mut out =
            refused(self, if slow { 3_000 } else { 300 }, AdminResult::Committed { log_seq });
        out.push((if slow { 3_300 } else { 1_000 }, event));
        out
    }

    fn member_action(
        &mut self,
        method: &str,
        group: &str,
        body: &JsonValue,
        now: Timestamp,
    ) -> Vec<Answer> {
        let anchor = body.get("anchor").and_then(JsonValue::as_str).unwrap_or_default().to_owned();
        let (display, ai_flag) = roster_person(&anchor);
        let name = display.clone().unwrap_or_default();
        let reply = |this: &Self, delay: u64, result: AdminResult| {
            this.outcome(method, body, result).map(|e| (delay, e))
        };
        // F3-4: the role of this account changed elsewhere.
        if group == "g-bookclub" && name == "Ana" && method != methods::MLS_GROUP_ROLE_SET {
            self.admin.my_role.insert(group.to_owned(), MlsRole::Member);
            return reply(
                self,
                300,
                AdminResult::Refused {
                    code: "not_admin".to_owned(),
                    retry_at: None,
                    head_log_seq: None,
                },
            )
            .into_iter()
            .collect();
        }
        *self.admin.commits.entry(group.to_owned()).or_default() += 1;
        let (kind, role) = match method {
            methods::MLS_GROUP_ROLE_SET => {
                let admin = body.get("role").and_then(JsonValue::as_str) == Some("admin");
                let role = if admin { MlsRole::Admin } else { MlsRole::Member };
                self.admin.roles.insert(anchor.clone(), role.clone());
                (MembershipKind::RoleChanged, Some(role))
            }
            methods::MLS_GROUP_MEMBER_BAN => {
                self.admin.gone.insert(anchor.clone());
                (MembershipKind::Banned, None)
            }
            _ => {
                self.admin.gone.insert(anchor.clone());
                (MembershipKind::Removed, None)
            }
        };
        let (log_seq, post) = self.commit_post(
            group,
            PostKind::Membership,
            PostBody::Membership {
                changes: vec![MembershipChange {
                    kind,
                    anchor: anchor.clone(),
                    display,
                    ai_flag,
                    role,
                    is_me: false,
                }],
                by: Some(Self::me()),
            },
            now,
        );
        let event =
            self.group_event(group, Some(log_seq), GroupEventKind::Membership(Box::new(post)));
        // The event of a role change of Ben comes before its reply (F3-3).
        let (reply_at, event_at) = if group == "g-design" && name == "Ben" {
            (600, 100)
        } else if (group == "g-design" && name == "Dana")
            || (group == "g-bookclub" && name == "Chen")
        {
            (3_000, 3_300)
        } else {
            (300, 1_000)
        };
        let mut out: Vec<Answer> =
            reply(self, reply_at, AdminResult::Committed { log_seq }).into_iter().collect();
        out.push((event_at, event));
        // F3-11: this account is removed while the call runs.
        if group == "g-bookclub" && name == "Chen" && method == methods::MLS_GROUP_MEMBER_REMOVE {
            let (log_seq, post) = self.commit_post(
                group,
                PostKind::Membership,
                PostBody::Membership {
                    changes: vec![MembershipChange {
                        kind: MembershipKind::Removed,
                        anchor: "anc-me".to_owned(),
                        display: Some("Me".to_owned()),
                        ai_flag: false,
                        role: None,
                        is_me: true,
                    }],
                    by: Some(author("anc-ana", Some("Ana"), false, false)),
                },
                now,
            );
            let removed =
                self.group_event(group, Some(log_seq), GroupEventKind::Membership(Box::new(post)));
            out.push((1_000, removed));
        }
        out
    }

    fn settings(
        &mut self,
        method: &str,
        group: &str,
        body: &JsonValue,
        now: Timestamp,
    ) -> Vec<Answer> {
        *self.admin.commits.entry(group.to_owned()).or_default() += 1;
        let name = body.get("name").and_then(JsonValue::as_str).map(str::to_owned);
        let approval = body.get("approval").and_then(JsonValue::as_bool);
        if let Some(name) = &name {
            self.admin.names.insert(group.to_owned(), name.clone());
        }
        if let Some(approval) = approval {
            self.admin.approvals.insert(group.to_owned(), approval);
        }
        let (log_seq, post) = self.commit_post(
            group,
            PostKind::Settings,
            PostBody::Settings { change: SettingsChange { name, approval }, by: Some(Self::me()) },
            now,
        );
        let event =
            self.group_event(group, Some(log_seq), GroupEventKind::Settings(Box::new(post)));
        let mut out: Vec<Answer> = self
            .outcome(method, body, AdminResult::Committed { log_seq })
            .map(|e| (300, e))
            .into_iter()
            .collect();
        out.push((1_000, event));
        if let Some(approval) = approval {
            // The SDK warns while approval is off and clears it when it is on.
            let warning = self.group_event(
                group,
                None,
                GroupEventKind::Warning {
                    code: WarningCode::AdmissionNeedsOnline,
                    cleared: approval,
                },
            );
            out.push((1_200, warning));
        }
        out
    }

    fn skip(&mut self, group: &str, body: &JsonValue, now: Timestamp) -> Vec<Answer> {
        let epoch = body.get("epoch").and_then(JsonValue::as_u64).unwrap_or(0);
        let from_seq = body.get("from_seq").and_then(JsonValue::as_u64).unwrap_or(0);
        let (reply_at, idle_after): (u64, i64) = match group {
            // F3-7 "late": the idle event comes first, then the reply.
            "g-latecatch" => (3_000, 1_000),
            "g-bookclub" => (3_000, 5_000),
            "g-catch0" => (500, 3_000),
            _ => (500, 5_000),
        };
        let entry = self.admin.catch.entry(group.to_owned()).or_insert((now.0 + idle_after, false));
        entry.1 = true;
        // The catch-up ends at the time the first refresh set, or
        // `idle_after` from this call, whichever comes first.
        if group == "g-latecatch" || entry.0 > now.0 + idle_after {
            entry.0 = now.0 + idle_after;
        }
        let idle_at = u64::try_from((entry.0 - now.0).max(100)).unwrap_or(100);
        let idle = self.group_event(
            group,
            None,
            GroupEventKind::CatchUp { state: CatchUpState::Idle, done: 0, total: 0 },
        );
        vec![
            (
                reply_at,
                DomainEvent::MlsSkipOutcome {
                    epoch,
                    group_id: group.to_owned(),
                    from_seq,
                    result: SkipResult::Rejoining,
                },
            ),
            (idle_at, idle),
        ]
    }

    fn void(&mut self, method: &str, group: &str, body: &JsonValue, now: Timestamp) -> Vec<Answer> {
        let target = body.get("log_seq").and_then(JsonValue::as_u64).unwrap_or(0);
        // "Stale head": a newer commit at 252 beats the undo (F3-10).
        if group == "g-stale" {
            let (log_seq, post) = {
                let log = self.log(group).max(252);
                let post = Post {
                    log_seq: Some(252),
                    post_id: format!("p-{group}-252"),
                    author: None,
                    appended_at: Some(now),
                    kind: PostKind::Membership,
                    body: PostBody::Membership {
                        changes: vec![MembershipChange {
                            kind: MembershipKind::Joined,
                            anchor: "anc-late".to_owned(),
                            display: Some("Fay".to_owned()),
                            ai_flag: false,
                            role: None,
                            is_me: false,
                        }],
                        by: Some(author("anc-ana", Some("Ana"), false, false)),
                    },
                    reply_to: None,
                    mentions: Vec::new(),
                    state: PostState::Sent,
                    send: None,
                };
                (log, post)
            };
            let _ = log_seq;
            let event =
                self.group_event(group, Some(252), GroupEventKind::Membership(Box::new(post)));
            let refused = self
                .outcome(
                    method,
                    body,
                    AdminResult::Refused {
                        code: "void_not_head".to_owned(),
                        retry_at: None,
                        head_log_seq: Some(252),
                    },
                )
                .map(|e| (400, e));
            return std::iter::once((100, event)).chain(refused).collect();
        }
        *self.admin.commits.entry(group.to_owned()).or_default() += 1;
        let (log_seq, post) = self.commit_post(
            group,
            PostKind::Void,
            PostBody::Void { voided_log_seq: Some(target), by: Some(Self::me()) },
            now,
        );
        let event = self.group_event(group, Some(log_seq), GroupEventKind::Void(Box::new(post)));
        let mut out: Vec<Answer> = self
            .outcome(method, body, AdminResult::Committed { log_seq })
            .map(|e| (400, e))
            .into_iter()
            .collect();
        out.push((1_000, event));
        out
    }

    /// The timed events of the first refresh of a group (F3-7 and F3-8).
    pub(crate) fn first_refresh(&mut self, group: &str, now: Timestamp) -> Vec<Answer> {
        let warning = |this: &mut Self, at: u64, code: &str, params: JsonValue, cleared: bool| {
            let code = WarningCode::parse(code, &params);
            (at, this.group_event(group, None, GroupEventKind::Warning { code, cleared }))
        };
        match group {
            // F3-7: catch-up progress, then idle after 5 s.
            "g-bookclub" => {
                self.admin.catch.insert(group.to_owned(), (now.0 + 5_000, false));
                let mut out: Vec<Answer> = (0..=20u64)
                    .map(|i| {
                        let event = self.group_event(
                            group,
                            None,
                            GroupEventKind::CatchUp {
                                state: CatchUpState::Running,
                                done: i * 6,
                                total: 120,
                            },
                        );
                        (i * 100, event)
                    })
                    .collect();
                let idle = self.group_event(
                    group,
                    None,
                    GroupEventKind::CatchUp { state: CatchUpState::Idle, done: 0, total: 0 },
                );
                out.push((5_000, idle));
                out
            }
            // F3-8: each warning in turn, then cleared; at the end three at once.
            "g-design" => {
                self.admin.fork = Some((group.to_owned(), now.0 + 1_000, now.0 + 4_000));
                let steps: [(&str, JsonValue); 12] = [
                    ("fork_alarm", json!({ "head_n": 2 })),
                    ("rejoin_needed", json!({ "reason": "pruned" })),
                    ("rejoin_needed", json!({ "reason": "gate_rejected" })),
                    ("rejoin_needed", json!({ "reason": "apply_failed" })),
                    ("rejoin_needed", json!({ "reason": "welcome_expired" })),
                    ("rejoin_needed", json!({ "reason": "state_lost" })),
                    ("rejoin_needed", json!({ "reason": "mystery" })),
                    ("undecryptable", json!({ "count": 12, "from_log_seq": 140 })),
                    ("admission_needs_online", json!({})),
                    ("host_unusual", json!({ "kind": "late_roll" })),
                    ("head_commit_rejected", json!({ "log_seq": 250 })),
                    ("weird", json!({})),
                ];
                let mut out = Vec::new();
                for (i, (code, params)) in steps.iter().enumerate() {
                    let at = 1_000 + 4_000 * i as u64;
                    out.push(warning(self, at, code, params.clone(), false));
                    out.push(warning(self, at + 3_000, code, params.clone(), true));
                }
                // Three together: order, two shown and "+1".
                let at = 1_000 + 4_000 * steps.len() as u64;
                out.push(warning(self, at, "host_unusual", json!({ "kind": "late_roll" }), false));
                out.push(warning(self, at + 100, "fork_alarm", json!({ "head_n": 3 }), false));
                out.push(warning(
                    self,
                    at + 200,
                    "undecryptable",
                    json!({ "count": 12, "from_log_seq": 140 }),
                    false,
                ));
                out
            }
            // F3-8: a member never sees `head_commit_rejected`.
            "g-neighbours" => {
                vec![warning(self, 1_000, "head_commit_rejected", json!({ "log_seq": 250 }), false)]
            }
            _ => Vec::new(),
        }
    }
}
