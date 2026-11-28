// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The MLS group answers of the demo (mock) source, active only while the
//! scenario `s12_mls_groups` is loaded (MLS GUI batch M2, fixtures F2-1 to
//! F2-10).
//!
//! Every `mls_group.*` call is answered from deterministic data built here,
//! before the scenario's response rules, the way `mock_cards` answers card
//! calls. The answers carry the ids of the call (`attempt`, `local_id`,
//! `group_id`, `post_id`), so a late answer never lands on another target.
//! Group ids are `g-design`, `g-neighbours`, `g-bookclub` and `g-scale`; the
//! other 116 groups are `g-fill-NNN`. Links are fake ASCII strings
//! `rfinv-*`, the group host is `node.example`, and the names are neutral
//! (Ana, Ben, Chen, Dana, and Scout as an agent). The large data (987
//! members, 20,000 posts) is generated on demand, one page at a time.
//!
//! The calls are counted per contract method (the developer panel shows
//! "mock calls: <method> N"), so a runner can tell that no call was made for
//! a link that failed the local check, that a double tap made one call, and
//! that nothing was sent without a Retry tap.
//!
//! Time-driven answers (the join that completes after 5 s, the delayed
//! reply of a revoke) are delays of the answer; the events of F2-7 follow the
//! first refresh of a group (opening it asks for one).

use std::collections::{BTreeMap, BTreeSet};

use ramflux_desktop_core::mls_group::{
    Author, CatchUp, DeleteResult, GroupEventKind, HistoryDirection, Invite, InviteLink,
    JoinOutcome, JoinRecord, JoinRefusal, LastPost, LeaveResult, MemberRow, MembershipChange,
    MembershipKind, MlsGroupSummary, MlsRole, NotSentReason, PendingReason, Post, PostBody,
    PostKind, PostState, PrivacyDelay, PushSetting, QueueReason, SendState, SettingsChange,
    Warning,
};
use ramflux_desktop_core::{DomainEvent, JsonValue, Timestamp, methods};
use serde_json::json;

/// The scenario that turns the handler on.
pub const SCENARIO: &str = "s12_mls_groups";

/// The group host.
const HOST: &str = "node.example";
/// Groups in the list.
const GROUPS: usize = 120;
/// Rows of a list page.
const LIST_PAGE: usize = 50;
/// Rows of a members page.
const MEMBERS_PAGE: usize = 200;

/// Answers after this many milliseconds.
type Answer = (u64, DomainEvent);

/// The answers of the demo MLS groups.
#[derive(Debug, Default)]
pub struct MockMls {
    calls: BTreeMap<String, u64>,
    sends: u64,
    deletes: u64,
    sent_posts: BTreeMap<String, BTreeMap<u64, Post>>,
    next_event_seq: u64,
    next_log: BTreeMap<String, u64>,
    members_failed_once: bool,
    refreshed: BTreeSet<String>,
    joined: BTreeSet<String>,
    left: BTreeSet<String>,
    created: Vec<(String, String, bool)>,
    invites: u64,
    revoked: BTreeSet<String>,
    read: BTreeMap<String, u64>,
}

/// A fixed set of neutral names.
const NAMES: [&str; 5] = ["Ana", "Ben", "Chen", "Dana", "Scout"];

fn author(anchor: &str, display: Option<&str>, ai: bool, me: bool) -> Author {
    Author {
        anchor: anchor.to_owned(),
        display: display.map(str::to_owned),
        ai_flag: ai,
        is_me: me,
    }
}

/// The nth author of the history of a group (cycles through the cast).
fn cast(n: u64) -> Author {
    match n % 8 {
        0 => author("anc-ana", Some("Ana"), false, false),
        1 => author("anc-ben", Some("Ben"), false, false),
        2 => author("anc-chen", Some("Chen"), false, false),
        3 => author("anc-dana", Some("Dana"), false, false),
        4 => author("anc-scout", Some("Scout"), true, false),
        5 => author("anc-me", Some("Me"), false, true),
        6 => author(&format!("anc-anon-{}", n % 5 + 1), None, false, false),
        _ => author("anc-ben", Some("Ben"), false, false),
    }
}

fn group_index(group: &str) -> Option<usize> {
    group.strip_prefix("g-fill-").and_then(|n| n.parse().ok())
}

/// The position of a group in the list order (newest activity first).
fn order_of(group: &str) -> usize {
    match group {
        "g-design" => 0,
        "g-neighbours" => 1,
        "g-bookclub" => 2,
        "g-scale" => 3,
        other => 3 + group_index(other).unwrap_or(0),
    }
}

/// Every group id of the list, in list order. `g-scale` is the 4th row.
fn all_ids() -> Vec<String> {
    let mut ids = vec![
        "g-design".to_owned(),
        "g-neighbours".to_owned(),
        "g-bookclub".to_owned(),
        "g-scale".to_owned(),
    ];
    ids.extend((1..=GROUPS - 4).map(|n| format!("g-fill-{n:03}")));
    ids
}

/// How many posts a group's history has.
fn post_count(group: &str) -> u64 {
    match group {
        "g-design" => 260,
        "g-scale" => 20_000,
        "g-neighbours" => 60,
        "g-bookclub" => 24,
        _ => 12,
    }
}

/// How many members a group has.
fn member_count(group: &str) -> u32 {
    match group {
        "g-design" => 12,
        "g-neighbours" => 987,
        "g-bookclub" => 3,
        "g-scale" => 1_000,
        other => 5 + u32::try_from(group_index(other).unwrap_or(0) % 30).unwrap_or(0),
    }
}

fn name_of(group: &str) -> String {
    match group {
        "g-design" => "Design".to_owned(),
        "g-neighbours" => "Neighbours".to_owned(),
        "g-bookclub" => "Book club".to_owned(),
        "g-scale" => "Scale test".to_owned(),
        other => match group_index(other) {
            Some(n) => format!("Group {n:03}"),
            None => other.to_owned(),
        },
    }
}

fn ts(base: Timestamp, back_ms: i64) -> Timestamp {
    Timestamp(base.0 - back_ms)
}

/// The post of a group at `log_seq`: a text post by the cast, with the
/// special rows of F2-3 in "Design".
fn post_at(group: &str, log_seq: u64, now: Timestamp) -> Post {
    let total = post_count(group);
    let appended =
        ts(now, i64::try_from(total.saturating_sub(log_seq)).unwrap_or(0) * 45_000 + 120_000);
    let base = |kind: PostKind, body: PostBody, who: Option<Author>| Post {
        log_seq: Some(log_seq),
        post_id: format!("p-{group}-{log_seq}"),
        author: who,
        appended_at: Some(appended),
        kind,
        body,
        reply_to: None,
        mentions: Vec::new(),
        state: PostState::Sent,
        send: None,
    };
    if group == "g-design" {
        match log_seq {
            30 => {
                let changes = (0..15)
                    .map(|i| MembershipChange {
                        kind: MembershipKind::Joined,
                        anchor: format!("anc-join-{i}"),
                        display: if i % 4 == 3 {
                            None
                        } else {
                            Some(format!("{} {}", NAMES[i % 4], i))
                        },
                        ai_flag: false,
                        role: None,
                        is_me: false,
                    })
                    .collect();
                return base(
                    PostKind::Membership,
                    PostBody::Membership {
                        changes,
                        by: Some(author("anc-ana", Some("Ana"), false, false)),
                    },
                    None,
                );
            }
            60 => {
                return base(
                    PostKind::Settings,
                    PostBody::Settings {
                        change: SettingsChange {
                            name: Some("Design team".to_owned()),
                            approval: None,
                        },
                        by: Some(author("anc-ana", Some("Ana"), false, false)),
                    },
                    None,
                );
            }
            90 => {
                return base(
                    PostKind::Void,
                    PostBody::Void {
                        voided_log_seq: Some(89),
                        by: Some(author("anc-ana", Some("Ana"), false, false)),
                    },
                    None,
                );
            }
            100 => {
                let mut post = base(
                    PostKind::Text,
                    PostBody::Opaque,
                    Some(author("anc-ben", Some("Ben"), false, false)),
                );
                post.state = PostState::Deleted;
                return post;
            }
            120 => {
                return base(
                    PostKind::Unknown("poll".to_owned()),
                    PostBody::Opaque,
                    Some(author("anc-dana", Some("Dana"), false, false)),
                );
            }
            _ => {}
        }
    }
    let who = cast(log_seq);
    let text = if who.ai_flag {
        format!("Scout: summary {log_seq}")
    } else {
        format!("Message {log_seq} in {}", name_of(group))
    };
    base(PostKind::Text, PostBody::Text(text), Some(who))
}

fn summary(group: &str, now: Timestamp, extra_unread: bool) -> MlsGroupSummary {
    let order = order_of(group);
    let members = member_count(group);
    let design = group == "g-design";
    let posts = post_count(group);
    let last = post_at(group, posts, now);
    MlsGroupSummary {
        group_id: group.to_owned(),
        name: name_of(group),
        tier: 1000,
        members,
        leaves: 0,
        agents: u32::from(design),
        approval: design || group == "g-bookclub",
        my_role: match group {
            "g-design" => MlsRole::Owner,
            "g-bookclub" => MlsRole::Admin,
            _ => MlsRole::Member,
        },
        host_node: HOST.to_owned(),
        push: PushSetting::MentionsReplies,
        catch_up: (group == "g-bookclub").then(|| CatchUp {
            state: "running".to_owned(),
            done: 40,
            total: 120,
        }),
        privacy_delay: PrivacyDelay::None,
        quota_post: None,
        quota_commit: None,
        last_log_seq: posts,
        warnings: vec![Warning {
            code: "host_ip_visible".to_owned(),
            params: json!({ "host": HOST }),
            cleared: false,
        }],
        last_activity_at: Some(ts(now, i64::try_from(order).unwrap_or(0) * 600_000 + 60_000)),
        unread: if design || extra_unread { 2 } else { 0 },
        unread_mentions: u32::from(design),
        read_log_seq: posts.saturating_sub(if design { 2 } else { 0 }),
        last_post: match last.body {
            PostBody::Text(ref text) => Some(LastPost {
                log_seq: posts,
                kind: "text".to_owned(),
                author: last.author.clone(),
                snippet: Some(text.chars().take(100).collect()),
                appended_at: last.appended_at,
            }),
            _ => None,
        },
    }
}

fn member_at(group: &str, i: u32) -> MemberRow {
    let neighbours = group == "g-neighbours";
    let (role, ai, me, nameless) = if group == "g-design" {
        match i {
            0 => (MlsRole::Owner, false, true, false),
            1 | 2 => (MlsRole::Admin, false, false, false),
            4 => (MlsRole::Member, true, false, false),
            _ => (MlsRole::Member, false, false, false),
        }
    } else if neighbours {
        (
            match i {
                0 => MlsRole::Owner,
                1..=8 => MlsRole::Admin,
                _ => MlsRole::Member,
            },
            matches!(i, 17 | 250 | 500 | 900),
            i == 42,
            i % 33 == 7,
        )
    } else {
        (
            match i {
                0 => MlsRole::Owner,
                1 => MlsRole::Admin,
                _ => MlsRole::Member,
            },
            false,
            i == 2,
            false,
        )
    };
    let display = if group == "g-design" && i < 5 {
        Some(["Me", "Ana", "Ben", "Chen", "Scout"][i as usize].to_owned())
    } else if nameless {
        None
    } else {
        Some(format!("{} {i:04}", NAMES[(i % 4) as usize]))
    };
    MemberRow {
        anchor: if me { "anc-me".to_owned() } else { format!("anc-{group}-{i:04}") },
        display,
        role,
        ai_flag: ai,
        devices: 1 + i % 3,
        is_me: me,
    }
}

/// An invite that expires `expires_in` milliseconds after `now`, valid for
/// `uses` uses, with the number of uses left unknown and not revoked.
fn invite(id: &str, link: &str, now: Timestamp, expires_in: i64, uses: u32) -> Invite {
    Invite {
        invite_id: id.to_owned(),
        link: InviteLink::new(link),
        max_uses: uses,
        uses_left: None,
        expires_at: Timestamp(now.0 + expires_in),
        revoked: false,
    }
}

impl MockMls {
    /// A handler with nothing called yet.
    pub fn new() -> Self {
        Self { next_event_seq: 1_000, ..Self::default() }
    }

    /// The calls made so far, per contract method.
    pub fn calls(&self) -> impl Iterator<Item = (&str, u64)> {
        self.calls.iter().map(|(method, count)| (method.as_str(), *count))
    }

    fn seq(&mut self) -> u64 {
        self.next_event_seq += 1;
        self.next_event_seq
    }

    fn log(&mut self, group: &str) -> u64 {
        let next = self.next_log.entry(group.to_owned()).or_insert_with(|| post_count(group));
        *next += 1;
        *next
    }

    fn group_event(
        &mut self,
        group: &str,
        log_seq: Option<u64>,
        kind: GroupEventKind,
    ) -> DomainEvent {
        DomainEvent::MlsGroupEvent { seq: self.seq(), group_id: group.to_owned(), log_seq, kind }
    }

    /// The events one `mls_group.*` call means, with their delays in
    /// milliseconds. `now` is the time of the call.
    pub fn respond(&mut self, method: &str, body: &JsonValue, now: Timestamp) -> Vec<Answer> {
        *self.calls.entry(method.to_owned()).or_default() += 1;
        let text = |field: &str| {
            body.get(field).and_then(JsonValue::as_str).unwrap_or_default().to_owned()
        };
        let group = text("group_id");
        match method {
            methods::MLS_GROUP_LIST => self.list(body, now),
            methods::MLS_GROUP_INFO => match self.summary(&group, now) {
                Some(summary) => {
                    vec![(80, DomainEvent::MlsGroupInfoRead { summary: Box::new(summary) })]
                }
                None => vec![(
                    80,
                    DomainEvent::MlsFailed {
                        method: method.to_owned(),
                        group_id: Some(group),
                        invite_id: None,
                        code: "not_member".to_owned(),
                    },
                )],
            },
            methods::MLS_GROUP_MEMBERS => self.members(&group, body),
            methods::MLS_GROUP_HISTORY => self.history(&group, body, now),
            methods::MLS_GROUP_SEND => self.send(&group, body, now),
            methods::MLS_GROUP_REFRESH => self.refresh(&group, now),
            methods::MLS_GROUP_MARK_READ => {
                let log_seq = body.get("log_seq").and_then(JsonValue::as_u64).unwrap_or(0);
                let read = self.read.entry(group.clone()).or_default();
                *read = (*read).max(log_seq);
                vec![(
                    50,
                    DomainEvent::MlsMarkedRead {
                        group_id: group,
                        read_log_seq: log_seq,
                        unread: 0,
                        unread_mentions: 0,
                    },
                )]
            }
            methods::MLS_GROUP_CREATE => self.create(body, now),
            methods::MLS_GROUP_INVITE_CREATE => {
                self.invites += 1;
                let uses = u32::try_from(body["max_uses"].as_u64().unwrap_or(100)).unwrap_or(100);
                let ttl = i64::try_from(body["ttl_s"].as_u64().unwrap_or(604_800)).unwrap_or(0);
                let n = self.invites;
                vec![(
                    300,
                    DomainEvent::MlsInviteCreated {
                        group_id: group,
                        invite: Invite {
                            uses_left: Some(uses),
                            ..invite(
                                &format!("i-new-{n}"),
                                &format!("rfinv-new-{n:04}-{}", "x".repeat(40)),
                                now,
                                ttl * 1_000,
                                uses,
                            )
                        },
                    },
                )]
            }
            methods::MLS_GROUP_INVITE_LIST => vec![(
                120,
                DomainEvent::MlsInvitesListed {
                    group_id: group,
                    invites: vec![
                        Invite {
                            revoked: self.revoked.contains("i-active"),
                            ..invite(
                                "i-active",
                                "rfinv-active-0001-aaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                                now,
                                5 * 86_400_000,
                                100,
                            )
                        },
                        Invite {
                            revoked: true,
                            ..invite(
                                "i-revoked",
                                "rfinv-revoked-0002-bbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                                now,
                                5 * 86_400_000,
                                10,
                            )
                        },
                        invite(
                            "i-expired",
                            "rfinv-expired-0003-cccccccccccccccccccccccccccc",
                            now,
                            -86_400_000,
                            1,
                        ),
                    ],
                },
            )],
            methods::MLS_GROUP_INVITE_REVOKE => {
                let id = text("invite_id");
                self.revoked.insert(id.clone());
                // The reply is delayed by 2 s: "Revoked" shows only after it.
                vec![(2_000, DomainEvent::MlsInviteRevoked { group_id: group, invite_id: id })]
            }
            methods::MLS_GROUP_JOIN => self.join(body, now),
            methods::MLS_GROUP_JOIN_PENDING => {
                vec![(60, DomainEvent::MlsJoinPendingListed { records: Vec::<JoinRecord>::new() })]
            }
            methods::MLS_GROUP_MESSAGE_DELETE => self.delete(&group, &text("post_id")),
            methods::MLS_GROUP_LEAVE => self.leave(&group),
            _ => Vec::new(),
        }
    }

    fn summary(&self, group: &str, now: Timestamp) -> Option<MlsGroupSummary> {
        let known = all_ids().iter().any(|g| g == group)
            || self.created.iter().any(|(g, _, _)| g == group)
            || self.joined.contains(group);
        if !known || self.left.contains(group) {
            return None;
        }
        if let Some((_, name, approval)) = self.created.iter().find(|(g, _, _)| g == group) {
            let mut summary = summary("g-fill-001", now, false);
            summary.group_id = group.to_owned();
            summary.name = name.clone();
            summary.approval = *approval;
            summary.my_role = MlsRole::Owner;
            summary.members = 1;
            summary.unread = 0;
            summary.unread_mentions = 0;
            summary.last_post = None;
            summary.last_log_seq = 0;
            summary.read_log_seq = 0;
            return Some(summary);
        }
        let base = if all_ids().iter().any(|g| g == group) { group } else { "g-fill-002" };
        let mut summary = summary(base, now, false);
        summary.group_id = group.to_owned();
        if base != group {
            summary.name = format!("Joined {group}");
        }
        if let Some(read) = self.read.get(group) {
            summary.read_log_seq = (*read).max(summary.read_log_seq);
            if *read >= summary.last_log_seq {
                summary.unread = 0;
                summary.unread_mentions = 0;
            }
        }
        Some(summary)
    }

    fn list(&mut self, body: &JsonValue, now: Timestamp) -> Vec<Answer> {
        let cursor = body.get("cursor").and_then(JsonValue::as_str).unwrap_or_default();
        let ids: Vec<String> = all_ids().into_iter().filter(|g| !self.left.contains(g)).collect();
        let mut extras: Vec<String> = self
            .created
            .iter()
            .map(|(g, _, _)| g.clone())
            .chain(self.joined.iter().cloned())
            .collect();
        extras.sort();
        // Pages of 50. Page 2 repeats a group of page 1 with a newer
        // activity time (F2-1); the order can change between pages.
        let (page, next, repeat): (Vec<String>, Option<&str>, bool) = match cursor {
            "" => {
                let mut page = extras;
                page.extend(ids.iter().take(LIST_PAGE.saturating_sub(page.len())).cloned());
                (page, Some("l2"), false)
            }
            "l2" => (
                ids.iter().skip(LIST_PAGE).take(LIST_PAGE - 1).cloned().collect(),
                Some("l3"),
                true,
            ),
            _ => (ids.iter().skip(2 * LIST_PAGE - 1).cloned().collect(), None, false),
        };
        let mut rows: Vec<MlsGroupSummary> = Vec::new();
        if repeat && let Some(repeated) = ids.get(10) {
            let mut row =
                self.summary(repeated, now).unwrap_or_else(|| summary(repeated, now, false));
            row.last_activity_at = Some(ts(now, 30_000));
            rows.push(row);
        }
        for group in page {
            if let Some(row) = self.summary(&group, now) {
                rows.push(row);
            }
        }
        vec![(
            100,
            DomainEvent::MlsGroupsListed {
                rows,
                cursor: next.map(str::to_owned),
                first_page: cursor.is_empty(),
            },
        )]
    }

    fn members(&mut self, group: &str, body: &JsonValue) -> Vec<Answer> {
        let cursor = body.get("cursor").and_then(JsonValue::as_str).unwrap_or_default();
        let page: u32 = match cursor {
            "" => 0,
            other => other.trim_start_matches('p').parse::<u32>().map_or(0, |n| n - 1),
        };
        // Page 3 of "Neighbours" fails once with host_unreachable (F2-2).
        if group == "g-neighbours" && page == 2 && !self.members_failed_once {
            self.members_failed_once = true;
            return vec![(
                200,
                DomainEvent::MlsMembersPage {
                    group_id: group.to_owned(),
                    rows: Vec::new(),
                    cursor: None,
                    failed: Some("host_unreachable".to_owned()),
                },
            )];
        }
        let total = member_count(group);
        let page_size = u32::try_from(MEMBERS_PAGE).unwrap_or(200);
        let from = page * page_size;
        let to = (from + page_size).min(total);
        let rows = (from..to).map(|i| member_at(group, i)).collect();
        let next = (to < total).then(|| format!("p{}", page + 2));
        vec![(
            150,
            DomainEvent::MlsMembersPage {
                group_id: group.to_owned(),
                rows,
                cursor: next,
                failed: None,
            },
        )]
    }

    fn history(&mut self, group: &str, body: &JsonValue, now: Timestamp) -> Vec<Answer> {
        let total = *self.next_log.get(group).unwrap_or(&post_count(group));
        let limit = body.get("limit").and_then(JsonValue::as_u64).unwrap_or(50);
        let before = body.get("before_log_seq").and_then(JsonValue::as_u64);
        let after = body.get("after_log_seq").and_then(JsonValue::as_u64);
        let (direction, lo, hi, more) = match (before, after) {
            (Some(b), _) => {
                let hi = b.saturating_sub(1);
                let lo = (hi + 1).saturating_sub(limit).max(1);
                (HistoryDirection::Before(b), lo, hi, lo > 1)
            }
            (None, Some(a)) => {
                let lo = a + 1;
                let hi = (a + limit).min(total);
                (HistoryDirection::After(a), lo, hi, hi < total)
            }
            _ => {
                let hi = total;
                let lo = (hi + 1).saturating_sub(limit).max(1);
                (HistoryDirection::Newest, lo, hi, lo > 1)
            }
        };
        let posts: Vec<Post> = (lo..=hi)
            .map(|ls| {
                self.sent_posts.get(group).and_then(|m| m.get(&ls)).cloned().unwrap_or_else(|| {
                    let mut post = post_at(group, ls, now);
                    post.log_seq = Some(ls);
                    post.post_id = format!("p-{group}-{ls}");
                    post
                })
            })
            .collect();
        vec![(
            180,
            DomainEvent::MlsHistoryPage {
                group_id: group.to_owned(),
                direction,
                posts,
                floor_log_seq: Some(1),
                last_log_seq: total,
                more,
            },
        )]
    }

    fn send(&mut self, group: &str, body: &JsonValue, now: Timestamp) -> Vec<Answer> {
        self.sends += 1;
        let local_id =
            body.get("local_id").and_then(JsonValue::as_str).unwrap_or_default().to_owned();
        let post_id = format!("p-send-{}", self.sends);
        let outcome = |state: SendState, post: Option<&str>| DomainEvent::MlsSendOutcome {
            group_id: group.to_owned(),
            local_id: local_id.clone(),
            post_id: post.map(str::to_owned),
            state,
        };
        // F2-4: the replies in turn, then plain sends.
        match self.sends {
            1 => {
                let log_seq = self.log(group);
                let sent = self.sent_post(&post_id, log_seq, body, now);
                self.sent_posts.entry(group.to_owned()).or_default().insert(log_seq, sent.clone());
                // The same post arrives again as an event (a repeat of a
                // loaded log position): it must not add a row.
                let repeat =
                    self.group_event(group, Some(log_seq), GroupEventKind::Post(Box::new(sent)));
                vec![(400, outcome(SendState::Sent { log_seq }, Some(&post_id))), (1_500, repeat)]
            }
            2 => {
                let log_seq = self.log(group);
                let event = self.group_event(
                    group,
                    Some(log_seq),
                    GroupEventKind::PostState {
                        post_id: post_id.clone(),
                        state: SendState::Sent { log_seq },
                    },
                );
                vec![
                    (400, outcome(SendState::Queued(QueueReason::CatchingUp), Some(&post_id))),
                    (3_000, event),
                ]
            }
            3 => {
                let event = self.group_event(
                    group,
                    None,
                    GroupEventKind::PostState {
                        post_id: post_id.clone(),
                        state: SendState::NotSent(NotSentReason::Failed),
                    },
                );
                vec![
                    (
                        400,
                        outcome(
                            SendState::Delayed { send_after: Timestamp(now.0 + 90_000) },
                            Some(&post_id),
                        ),
                    ),
                    (5_000, event),
                ]
            }
            4 => vec![(
                400,
                outcome(
                    SendState::NotSent(NotSentReason::RateLimited {
                        resets_at: Some(Timestamp(now.0 + 20_000)),
                        first_day: true,
                        limit: 20,
                        remaining: 0,
                    }),
                    None,
                ),
            )],
            5 => vec![(
                400,
                outcome(
                    SendState::NotSent(NotSentReason::TooLarge { max_bytes: Some(65_536) }),
                    None,
                ),
            )],
            // An unknown state the client does not know reads as queued.
            6 => vec![(400, outcome(SendState::Queued(QueueReason::Unknown), Some(&post_id)))],
            _ => {
                let log_seq = self.log(group);
                vec![(400, outcome(SendState::Sent { log_seq }, Some(&post_id)))]
            }
        }
    }

    fn sent_post(&self, post_id: &str, log_seq: u64, body: &JsonValue, now: Timestamp) -> Post {
        Post {
            log_seq: Some(log_seq),
            post_id: post_id.to_owned(),
            author: Some(author("anc-me", Some("Me"), false, true)),
            appended_at: Some(now),
            kind: PostKind::Text,
            body: PostBody::Text(
                body.get("text").and_then(JsonValue::as_str).unwrap_or_default().to_owned(),
            ),
            reply_to: None,
            mentions: Vec::new(),
            state: PostState::Sent,
            send: None,
        }
    }

    fn refresh(&mut self, group: &str, now: Timestamp) -> Vec<Answer> {
        let mut out = vec![(
            120,
            DomainEvent::MlsRefreshed {
                group_id: group.to_owned(),
                last_log_seq: *self.next_log.get(group).unwrap_or(&post_count(group)),
            },
        )];
        // F2-7: the first refresh of a group brings its news once.
        if self.refreshed.insert(group.to_owned()) {
            out.extend(self.news(group, now));
        }
        out
    }

    /// The membership and settings events of F2-7.
    fn news(&mut self, group: &str, now: Timestamp) -> Vec<Answer> {
        let change =
            |kind: MembershipKind, anchor: &str, display: &str, me: bool, role: Option<MlsRole>| {
                MembershipChange {
                    kind,
                    anchor: anchor.to_owned(),
                    display: Some(display.to_owned()),
                    ai_flag: false,
                    role,
                    is_me: me,
                }
            };
        let membership = |this: &mut Self, changes: Vec<MembershipChange>, by: Option<Author>| {
            let log_seq = this.log(group);
            let post = Post {
                log_seq: Some(log_seq),
                post_id: format!("p-{group}-{log_seq}"),
                author: None,
                appended_at: Some(now),
                kind: PostKind::Membership,
                body: PostBody::Membership { changes, by },
                reply_to: None,
                mentions: Vec::new(),
                state: PostState::Sent,
                send: None,
            };
            this.group_event(group, Some(log_seq), GroupEventKind::Membership(Box::new(post)))
        };
        let ana = Some(author("anc-ana", Some("Ana"), false, false));
        match group {
            "g-design" => {
                let removed = membership(
                    self,
                    vec![change(MembershipKind::Removed, "anc-chen", "Chen", false, None)],
                    ana.clone(),
                );
                let banned = membership(
                    self,
                    vec![change(MembershipKind::Banned, "anc-dana", "Dana", false, None)],
                    ana.clone(),
                );
                let role = membership(
                    self,
                    vec![change(
                        MembershipKind::RoleChanged,
                        "anc-ben",
                        "Ben",
                        false,
                        Some(MlsRole::Admin),
                    )],
                    ana.clone(),
                );
                let device = membership(
                    self,
                    vec![change(MembershipKind::DeviceAdded, "anc-ben", "Ben", false, None)],
                    None,
                );
                let log_seq = self.log(group);
                let settings_post = Post {
                    log_seq: Some(log_seq),
                    post_id: format!("p-{group}-{log_seq}"),
                    author: None,
                    appended_at: Some(now),
                    kind: PostKind::Settings,
                    body: PostBody::Settings {
                        change: SettingsChange { name: None, approval: Some(false) },
                        by: ana,
                    },
                    reply_to: None,
                    mentions: Vec::new(),
                    state: PostState::Sent,
                    send: None,
                };
                let settings = self.group_event(
                    group,
                    Some(log_seq),
                    GroupEventKind::Settings(Box::new(settings_post)),
                );
                vec![
                    (1_500, removed),
                    (3_000, banned),
                    (4_500, role),
                    (6_000, device),
                    (7_500, settings),
                ]
            }
            "g-bookclub" => {
                let removed = membership(
                    self,
                    vec![change(MembershipKind::Removed, "anc-me", "Me", true, None)],
                    ana,
                );
                vec![(3_000, removed)]
            }
            _ => Vec::new(),
        }
    }

    fn create(&mut self, body: &JsonValue, now: Timestamp) -> Vec<Answer> {
        let name = body.get("name").and_then(JsonValue::as_str).unwrap_or_default().to_owned();
        let approval = body.get("approval").and_then(JsonValue::as_bool).unwrap_or(true);
        // A name that reads "full" shows the typed refusal.
        if name.to_lowercase().contains("refuse") {
            return vec![(
                500,
                DomainEvent::MlsCreateFailed { code: "tier_unavailable".to_owned() },
            )];
        }
        let group = format!("g-created-{}", self.created.len() + 1);
        self.created.push((group.clone(), name.clone(), approval));
        vec![(
            600,
            DomainEvent::MlsCreated {
                group_id: group.clone(),
                name,
                approval,
                invite: Invite {
                    uses_left: Some(100),
                    ..invite(
                        "i-created",
                        &format!("rfinv-created-{:04}-{}", self.created.len(), "k".repeat(40)),
                        now,
                        7 * 86_400_000,
                        100,
                    )
                },
            },
        )]
    }

    fn join(&mut self, body: &JsonValue, now: Timestamp) -> Vec<Answer> {
        let attempt = body.get("attempt").and_then(JsonValue::as_u64).unwrap_or(0);
        let link = body.get("link").and_then(JsonValue::as_str).unwrap_or_default().to_owned();
        let outcome = |delay: u64, outcome: JoinOutcome| {
            (delay, DomainEvent::MlsJoinOutcome { attempt, outcome })
        };
        let pending =
            |request: &str, reason: PendingReason, group: &str, days: i64| JoinOutcome::Pending {
                request_id: request.to_owned(),
                reason,
                expires_at: Some(Timestamp(now.0 + days * 86_400_000)),
                group_id: Some(group.to_owned()),
            };
        let refused = |refusal: JoinRefusal| JoinOutcome::Refused(refusal);
        match link.as_str() {
            "rfinv-joined" => {
                self.joined.insert("g-joined".to_owned());
                vec![outcome(600, JoinOutcome::Joined { group_id: "g-joined".to_owned() })]
            }
            "rfinv-admin" => {
                vec![outcome(600, pending("r-admin", PendingReason::AwaitingAdmin, "g-admin", 3))]
            }
            "rfinv-online" => {
                self.joined.insert("g-online".to_owned());
                let result = self.group_event(
                    "g-online",
                    None,
                    GroupEventKind::JoinResult {
                        request_id: "r-online".to_owned(),
                        group_id: "g-online".to_owned(),
                        result: Ok(()),
                    },
                );
                vec![
                    outcome(
                        600,
                        pending("r-online", PendingReason::AwaitingMemberOnline, "g-online", 1),
                    ),
                    // The request completes 5 s after the call.
                    (5_000, result),
                ]
            }
            "rfinv-slow" => {
                self.joined.insert("g-slow".to_owned());
                vec![outcome(4_000, JoinOutcome::Joined { group_id: "g-slow".to_owned() })]
            }
            "rfinv-down" => {
                vec![outcome(500, JoinOutcome::Failed { code: "host_unreachable".to_owned() })]
            }
            "rfinv-unknownreason" => {
                vec![outcome(500, refused(JoinRefusal::Unknown("banana".to_owned())))]
            }
            "rfinv-refused-link_invalid" => vec![outcome(300, refused(JoinRefusal::LinkInvalid))],
            "rfinv-refused-invite_expired" => vec![outcome(
                300,
                refused(JoinRefusal::InviteExpired {
                    expired_at: Some(Timestamp(now.0 - 86_400_000)),
                }),
            )],
            "rfinv-refused-invite_invalid" => {
                vec![outcome(300, refused(JoinRefusal::InviteInvalid))]
            }
            "rfinv-refused-group_full" => {
                vec![outcome(300, refused(JoinRefusal::GroupFull { leaf_cap: Some(3_000) }))]
            }
            "rfinv-refused-queue_full" => vec![outcome(300, refused(JoinRefusal::QueueFull))],
            "rfinv-refused-request_expired" => vec![outcome(
                300,
                refused(JoinRefusal::RequestExpired {
                    expired_at: Some(Timestamp(now.0 - 3_600_000)),
                }),
            )],
            "rfinv-refused-already_member" => vec![outcome(
                300,
                refused(JoinRefusal::AlreadyMember { group_id: Some("g-design".to_owned()) }),
            )],
            "rfinv-refused-rate_limited" => {
                vec![outcome(300, refused(JoinRefusal::RateLimited { retry_after_s: Some(10) }))]
            }
            _ => vec![outcome(300, refused(JoinRefusal::InviteInvalid))],
        }
    }

    fn delete(&mut self, group: &str, post_id: &str) -> Vec<Answer> {
        self.deletes += 1;
        // The second delete is refused as if the post were a foreign one.
        if self.deletes == 2 {
            return vec![(
                500,
                DomainEvent::MlsDeleteOutcome {
                    group_id: group.to_owned(),
                    post_id: post_id.to_owned(),
                    result: DeleteResult::Failed { code: "not_author".to_owned() },
                },
            )];
        }
        let log_seq = self.log(group);
        let tombstone = self.group_event(
            group,
            Some(log_seq),
            GroupEventKind::PostDeleted {
                post_id: post_id.to_owned(),
                post_log_seq: post_id.rsplit('-').next().and_then(|n| n.parse().ok()),
                log_seq: Some(log_seq),
            },
        );
        vec![
            (
                2_000,
                DomainEvent::MlsDeleteOutcome {
                    group_id: group.to_owned(),
                    post_id: post_id.to_owned(),
                    result: DeleteResult::Committed { log_seq },
                },
            ),
            // The event repeats the tombstone once: it shows once.
            (3_000, tombstone),
        ]
    }

    fn leave(&mut self, group: &str) -> Vec<Answer> {
        // The owner cannot leave (F2-8).
        if group == "g-design" {
            return vec![(
                400,
                DomainEvent::MlsLeft {
                    group_id: group.to_owned(),
                    result: LeaveResult::OwnerProtected,
                },
            )];
        }
        self.left.insert(group.to_owned());
        let log_seq = self.log(group);
        let post = Post {
            log_seq: Some(log_seq),
            post_id: format!("p-{group}-{log_seq}"),
            author: None,
            appended_at: None,
            kind: PostKind::Membership,
            body: PostBody::Membership {
                changes: vec![MembershipChange {
                    kind: MembershipKind::Left,
                    anchor: "anc-me".to_owned(),
                    display: Some("Me".to_owned()),
                    ai_flag: false,
                    role: None,
                    is_me: true,
                }],
                by: None,
            },
            reply_to: None,
            mentions: Vec::new(),
            state: PostState::Sent,
            send: None,
        };
        let left =
            self.group_event(group, Some(log_seq), GroupEventKind::Membership(Box::new(post)));
        vec![
            (
                500,
                DomainEvent::MlsLeft { group_id: group.to_owned(), result: LeaveResult::Committed },
            ),
            (1_500, left),
        ]
    }
}
