// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! MLS groups: replies, error frames and `group.event` frames to domain
//! events (method table r4).
//!
//! Pure functions: nothing here calls, keeps or resends anything. Times are
//! unix milliseconds (`*_at`), durations seconds (`*_s`). A token this build
//! does not know becomes the `Unknown` case of its family, with one log line
//! that carries the token only (never a name, a link, an id or a body).
//!
//! | wire | domain event |
//! |---|---|
//! | `group.list` `{groups[], cursor?}` | `MlsGroupsListed` |
//! | `group.info` | `MlsGroupInfoRead` |
//! | `group.members` `{rows[], cursor?}` | `MlsMembersPage` (a failed page keeps its code) |
//! | `group.history` `{posts[], floor_log_seq, last_log_seq, more}` | `MlsHistoryPage` |
//! | `group.create` `{group_id, invite}` | `MlsCreated`, else `MlsCreateFailed` |
//! | `group.invite.create` / `.list` / `.revoke` | `MlsInviteCreated` / `MlsInvitesListed` / `MlsInviteRevoked` |
//! | `group.join` `joined` / `pending` / `refused` | `MlsJoinOutcome` (an error frame is `Failed`) |
//! | `group.join.pending` `{requests[]}` | `MlsJoinPendingListed` |
//! | `group.send` `{post_id, state, params}` | `MlsSendOutcome` (an error frame is `NotSent`) |
//! | `group.message.delete` | `MlsDeleteOutcome` |
//! | `group.leave` | `MlsLeft` |
//! | `group.refresh` `{groups[{group_id, last_log_seq}]}` | `MlsRefreshed` |
//! | `group.mark_read` | `MlsMarkedRead` |
//! | event `group.event` `post`, `post_state`, `post_deleted`, `membership`, `settings`, `join_result`, `read` | `MlsGroupEvent` |
//!
//! Anything else on `group.event` (`pending_join`, `catch_up`, `warning`,
//! `void`) is skipped without logging its payload.

use ramflux_desktop_core::mls_group::{
    Author, CatchUp, DeleteResult, GroupEventKind, HistoryDirection, Invite, InviteLink,
    JoinOutcome, JoinRecord, JoinRefusal, LastPost, LeaveResult, MemberRow, MembershipChange,
    MembershipKind, MlsGroupSummary, MlsRole, NotSentReason, PendingReason, Post, PostBody,
    PostKind, PostState, PrivacyDelay, PushSetting, QueueReason, Quota, SendState, SettingsChange,
    Warning,
};
use ramflux_desktop_core::{DomainEvent, Timestamp, methods};
use serde_json::Value;

use crate::bus::BusError;
use crate::local::app_log;

/// The topic of the MLS group events.
pub const GROUP_EVENT: &str = "group.event";

fn str_of<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}

fn string(value: &Value, field: &str) -> String {
    str_of(value, field).unwrap_or_default().to_owned()
}

fn opt_string(value: &Value, field: &str) -> Option<String> {
    str_of(value, field).filter(|s| !s.is_empty()).map(str::to_owned)
}

fn number(value: &Value, field: &str) -> u64 {
    value.get(field).and_then(Value::as_u64).unwrap_or(0)
}

fn small(value: &Value, field: &str) -> u32 {
    u32::try_from(number(value, field)).unwrap_or(u32::MAX)
}

fn time(value: &Value, field: &str) -> Option<Timestamp> {
    value.get(field).and_then(Value::as_i64).map(Timestamp)
}

fn flag(value: &Value, field: &str) -> bool {
    value.get(field).and_then(Value::as_bool).unwrap_or(false)
}

fn array<'a>(value: &'a Value, field: &str) -> &'a [Value] {
    value.get(field).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn unknown(family: &str, token: &str) {
    app_log::warn(&format!("unknown {family}: {token}"));
}

/// The code of an error frame: its typed reason, else its code.
pub fn error_code(error: &BusError) -> String {
    match error {
        BusError::Daemon { reason_code: Some(reason), .. } => reason.clone(),
        BusError::Daemon { code, .. } => code.clone(),
        BusError::Timeout => "host_unreachable".to_owned(),
        BusError::Transport(_) => "host_unreachable".to_owned(),
    }
}

fn error_details(error: &BusError) -> Value {
    match error {
        BusError::Daemon { details: Some(details), .. } => (**details).clone(),
        _ => Value::Null,
    }
}

fn role(token: &str) -> MlsRole {
    match token {
        "owner" => MlsRole::Owner,
        "admin" => MlsRole::Admin,
        "member" => MlsRole::Member,
        other => {
            unknown("role", other);
            MlsRole::Unknown(other.to_owned())
        }
    }
}

fn push(token: &str) -> PushSetting {
    match token {
        "mentions_replies" | "mentions" => PushSetting::MentionsReplies,
        "all" => PushSetting::All,
        "none" => PushSetting::None,
        other => {
            unknown("push", other);
            PushSetting::Unknown(other.to_owned())
        }
    }
}

/// An author object; a bare anchor string is read as an author with no name.
pub fn author(value: &Value) -> Option<Author> {
    if let Some(anchor) = value.as_str() {
        return Some(Author {
            anchor: anchor.to_owned(),
            display: None,
            ai_flag: false,
            is_me: false,
        });
    }
    let anchor = opt_string(value, "anchor")?;
    Some(Author {
        anchor,
        display: opt_string(value, "display"),
        ai_flag: flag(value, "ai_flag"),
        is_me: flag(value, "is_me"),
    })
}

fn quota(value: &Value) -> Option<Quota> {
    value.is_object().then(|| Quota {
        limit: number(value, "limit"),
        remaining: number(value, "remaining"),
        first_day: flag(value, "first_day"),
        resets_at: time(value, "resets_at"),
    })
}

/// A group as `group.list` and `group.info` describe it.
pub fn summary(value: &Value) -> Option<MlsGroupSummary> {
    let group_id = opt_string(value, "group_id")?;
    let privacy_delay = match value.get("privacy_delay") {
        None | Some(Value::Null) => PrivacyDelay::None,
        Some(delay) => time(delay, "send_after").map_or(PrivacyDelay::Armed, PrivacyDelay::Waiting),
    };
    let quotas = value.get("quota").cloned().unwrap_or(Value::Null);
    Some(MlsGroupSummary {
        group_id,
        name: string(value, "name"),
        tier: small(value, "tier"),
        members: small(value, "members"),
        leaves: small(value, "leaves"),
        agents: small(value, "agents"),
        approval: flag(value, "approval"),
        my_role: role(str_of(value, "my_role").unwrap_or("member")),
        host_node: string(value, "host_node"),
        push: push(str_of(value, "push").unwrap_or("mentions_replies")),
        catch_up: value.get("catch_up").filter(|c| c.is_object()).map(|c| CatchUp {
            state: string(c, "state"),
            done: number(c, "done"),
            total: number(c, "total"),
        }),
        privacy_delay,
        quota_post: quotas.get("post").and_then(quota),
        quota_commit: quotas.get("commit").and_then(quota),
        last_log_seq: number(value, "last_log_seq"),
        warnings: array(value, "warnings")
            .iter()
            .map(|w| Warning {
                code: string(w, "code"),
                params: w.get("params").cloned().unwrap_or(Value::Null),
                cleared: flag(w, "cleared"),
            })
            .collect(),
        last_activity_at: time(value, "last_activity_at"),
        unread: small(value, "unread"),
        unread_mentions: small(value, "unread_mentions"),
        read_log_seq: number(value, "read_log_seq"),
        last_post: value.get("last_post").filter(|p| p.is_object()).map(|p| LastPost {
            log_seq: number(p, "log_seq"),
            kind: string(p, "kind"),
            author: p.get("author").and_then(author),
            snippet: opt_string(p, "snippet"),
            appended_at: time(p, "appended_at"),
        }),
    })
}

fn member(value: &Value) -> Option<MemberRow> {
    Some(MemberRow {
        anchor: opt_string(value, "anchor")?,
        display: opt_string(value, "display"),
        role: role(str_of(value, "role").unwrap_or("member")),
        ai_flag: flag(value, "ai_flag"),
        devices: small(value, "devices"),
        is_me: flag(value, "is_me"),
    })
}

/// An invite row or reply.
pub fn invite(value: &Value) -> Option<Invite> {
    Some(Invite {
        invite_id: opt_string(value, "invite_id")?,
        link: InviteLink::new(string(value, "link")),
        max_uses: small(value, "max_uses"),
        uses_left: value.get("uses_left").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()),
        expires_at: time(value, "expires_at").unwrap_or_default(),
        revoked: flag(value, "revoked"),
    })
}

fn queue_reason(params: &Value) -> QueueReason {
    match str_of(params, "reason").unwrap_or_default() {
        "catching_up" => QueueReason::CatchingUp,
        "host_unreachable" => QueueReason::HostUnreachable,
        "fetching_tokens" => QueueReason::FetchingTokens,
        other => {
            unknown("queue reason", other);
            QueueReason::Unknown
        }
    }
}

/// The state of a send: `sent{log_seq}`, `queued{reason}`,
/// `delayed_privacy{send_after}`, `rate_limited{..}`, `failed`.
pub fn send_state(state: &str, params: &Value, log_seq: Option<u64>) -> SendState {
    match state {
        "sent" => params
            .get("log_seq")
            .and_then(Value::as_u64)
            .or(log_seq)
            .map_or(SendState::Queued(QueueReason::Unknown), |log_seq| SendState::Sent { log_seq }),
        "queued" | "pending" => SendState::Queued(queue_reason(params)),
        "delayed_privacy" => time(params, "send_after")
            .map_or(SendState::Queued(QueueReason::Unknown), |send_after| SendState::Delayed {
                send_after,
            }),
        "rate_limited" => SendState::NotSent(NotSentReason::from_code("rate_limited", params)),
        "failed" => SendState::NotSent(NotSentReason::Failed),
        other => {
            unknown("send state", other);
            SendState::Queued(QueueReason::Unknown)
        }
    }
}

fn post_kind(token: &str) -> PostKind {
    match token {
        "text" => PostKind::Text,
        "attachment" => PostKind::Attachment,
        "card" => PostKind::Card,
        "membership" => PostKind::Membership,
        "settings" => PostKind::Settings,
        "void" => PostKind::Void,
        other => {
            unknown("post kind", other);
            PostKind::Unknown(other.to_owned())
        }
    }
}

fn post_state(token: &str) -> PostState {
    match token {
        "sent" => PostState::Sent,
        "pending" => PostState::Pending,
        "queued" => PostState::Queued,
        "delayed_privacy" => PostState::DelayedPrivacy,
        "rate_limited" => PostState::RateLimited,
        "failed" => PostState::Failed,
        "deleted" => PostState::Deleted,
        other => {
            unknown("post state", other);
            PostState::Unknown(other.to_owned())
        }
    }
}

fn membership_kind(token: &str) -> MembershipKind {
    match token {
        "joined" => MembershipKind::Joined,
        "left" => MembershipKind::Left,
        "removed" => MembershipKind::Removed,
        "banned" => MembershipKind::Banned,
        "role_changed" => MembershipKind::RoleChanged,
        "device_added" => MembershipKind::DeviceAdded,
        "device_removed" => MembershipKind::DeviceRemoved,
        other => {
            unknown("membership change", other);
            MembershipKind::Unknown(other.to_owned())
        }
    }
}

/// A history row (also the body of a `post`, `membership` or `settings` event).
pub fn post(value: &Value) -> Option<Post> {
    let post_id = opt_string(value, "post_id")?;
    let kind = post_kind(str_of(value, "kind").unwrap_or("text"));
    let raw = value.get("body").cloned().unwrap_or(Value::Null);
    let by = || raw.get("by").and_then(author);
    let body = match &kind {
        PostKind::Text => PostBody::Text(raw.as_str().map(str::to_owned).unwrap_or_default()),
        PostKind::Membership => PostBody::Membership {
            changes: array(&raw, "changes")
                .iter()
                .filter_map(|c| {
                    Some(MembershipChange {
                        kind: membership_kind(str_of(c, "change")?),
                        anchor: opt_string(c, "anchor")?,
                        display: opt_string(c, "display"),
                        ai_flag: flag(c, "ai_flag"),
                        role: str_of(c, "role").map(role),
                        is_me: flag(c, "is_me"),
                    })
                })
                .collect(),
            by: by(),
        },
        PostKind::Settings => PostBody::Settings {
            change: SettingsChange {
                name: opt_string(&raw, "name"),
                approval: raw.get("approval").and_then(Value::as_bool),
            },
            by: by(),
        },
        PostKind::Void => PostBody::Void {
            voided_log_seq: raw.get("voided_log_seq").and_then(Value::as_u64),
            by: by(),
        },
        _ => PostBody::Opaque,
    };
    let state = post_state(str_of(value, "state").unwrap_or("sent"));
    let send = value.get("send").filter(|s| s.is_object()).map(|s| {
        send_state(
            str_of(s, "state").unwrap_or("queued"),
            s.get("params").unwrap_or(&Value::Null),
            None,
        )
    });
    Some(Post {
        log_seq: value.get("log_seq").and_then(Value::as_u64),
        post_id,
        author: value.get("author").and_then(author),
        appended_at: time(value, "appended_at"),
        kind,
        body,
        reply_to: opt_string(value, "reply_to"),
        mentions: array(value, "mentions")
            .iter()
            .filter_map(|m| m.as_str().map(str::to_owned))
            .collect(),
        state,
        send,
    })
}

fn pending_reason(token: &str) -> PendingReason {
    match token {
        "awaiting_admin" => PendingReason::AwaitingAdmin,
        "awaiting_member_online" => PendingReason::AwaitingMemberOnline,
        other => {
            unknown("pending reason", other);
            PendingReason::Unknown(other.to_owned())
        }
    }
}

/// A join refusal: the reason and its parameters.
pub fn join_refusal(reason: &str, params: &Value) -> JoinRefusal {
    match reason {
        "link_invalid" => JoinRefusal::LinkInvalid,
        "invite_expired" => JoinRefusal::InviteExpired { expired_at: time(params, "expired_at") },
        "invite_invalid" => JoinRefusal::InviteInvalid,
        "group_full" => JoinRefusal::GroupFull {
            leaf_cap: params.get("leaf_cap").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()),
        },
        "queue_full" => JoinRefusal::QueueFull,
        "request_expired" => JoinRefusal::RequestExpired { expired_at: time(params, "expired_at") },
        "already_member" => JoinRefusal::AlreadyMember { group_id: opt_string(params, "group_id") },
        "rate_limited" => {
            JoinRefusal::RateLimited { retry_after_s: params.get("retry_after_s").and_then(Value::as_u64) }
        }
        other => {
            unknown("join refusal", other);
            JoinRefusal::Unknown(other.to_owned())
        }
    }
}

/// The outcome of a `group.join` reply.
pub fn join_outcome(reply: &Value) -> JoinOutcome {
    let state = str_of(reply, "state").or_else(|| str_of(reply, "result")).unwrap_or_default();
    match state {
        "joined" => JoinOutcome::Joined { group_id: string(reply, "group_id") },
        "pending" => JoinOutcome::Pending {
            request_id: string(reply, "request_id"),
            reason: pending_reason(str_of(reply, "reason").unwrap_or_default()),
            expires_at: time(reply, "expires_at"),
            group_id: opt_string(reply, "group_id"),
        },
        "refused" => JoinOutcome::Refused(join_refusal(
            str_of(reply, "reason").unwrap_or_default(),
            reply.get("params").unwrap_or(&Value::Null),
        )),
        other => {
            unknown("join state", other);
            JoinOutcome::Refused(JoinRefusal::Unknown(other.to_owned()))
        }
    }
}

fn join_record(value: &Value) -> Option<JoinRecord> {
    Some(JoinRecord {
        request_id: opt_string(value, "request_id")?,
        reason: pending_reason(str_of(value, "reason").unwrap_or_default()),
        expires_at: time(value, "expires_at"),
        group_id: opt_string(value, "group_id"),
        attempt: 0,
    })
}

/// A `group.event` frame as the domain event it stands for, `None` for a
/// kind M2 skips.
pub fn group_event(frame: &Value) -> Option<DomainEvent> {
    let group_id = opt_string(frame, "group_id")?;
    let seq = number(frame, "seq");
    let log_seq = frame.get("log_seq").and_then(Value::as_u64);
    let row = frame.get("post").unwrap_or(frame);
    let kind = match str_of(frame, "kind")? {
        "post" => GroupEventKind::Post(Box::new(post(row)?)),
        "membership" => {
            let mut row = post(row)?;
            row.kind = PostKind::Membership;
            GroupEventKind::Membership(Box::new(row))
        }
        "settings" => {
            let mut row = post(row)?;
            row.kind = PostKind::Settings;
            GroupEventKind::Settings(Box::new(row))
        }
        "post_state" => GroupEventKind::PostState {
            post_id: opt_string(frame, "post_id")?,
            state: send_state(
                str_of(frame, "state")?,
                frame.get("params").unwrap_or(&Value::Null),
                log_seq,
            ),
        },
        "post_deleted" => GroupEventKind::PostDeleted {
            post_id: opt_string(frame, "post_id")?,
            post_log_seq: frame.get("post_log_seq").and_then(Value::as_u64),
            log_seq,
        },
        "join_result" => {
            let params = frame.get("params").unwrap_or(&Value::Null);
            let result = match str_of(frame, "state") {
                Some("joined") => Ok(()),
                _ => Err(join_refusal(str_of(frame, "reason").unwrap_or("request_expired"), params)),
            };
            GroupEventKind::JoinResult {
                request_id: opt_string(frame, "request_id")?,
                group_id: group_id.clone(),
                result,
            }
        }
        "read" => GroupEventKind::Read {
            read_log_seq: number(frame, "read_log_seq"),
            unread: small(frame, "unread"),
            unread_mentions: small(frame, "unread_mentions"),
        },
        // `pending_join`, `catch_up`, `warning`, `void` and anything else
        // are skipped, without their payload.
        _ => return None,
    };
    Some(DomainEvent::MlsGroupEvent { seq, group_id, log_seq, kind })
}

/// What a finished call of an MLS group method says to the store.
pub fn outcome(method: &str, body: &Value, result: &Result<Value, BusError>) -> Vec<DomainEvent> {
    let group = || opt_string(body, "group_id");
    let failed = |error: &BusError| DomainEvent::MlsFailed {
        method: method.to_owned(),
        group_id: group(),
        invite_id: opt_string(body, "invite_id"),
        code: error_code(error),
    };
    let reply = match result {
        Ok(reply) => reply,
        Err(error) => return vec![error_outcome(method, body, error).unwrap_or_else(|| failed(error))],
    };
    let event = match method {
        methods::MLS_GROUP_LIST => DomainEvent::MlsGroupsListed {
            rows: array(reply, "groups").iter().filter_map(summary).collect(),
            cursor: opt_string(reply, "cursor"),
            first_page: body.get("cursor").is_none_or(Value::is_null),
        },
        methods::MLS_GROUP_INFO => match summary(reply) {
            Some(summary) => DomainEvent::MlsGroupInfoRead { summary: Box::new(summary) },
            None => return Vec::new(),
        },
        methods::MLS_GROUP_CREATE => match (opt_string(reply, "group_id"), reply.get("invite").and_then(invite)) {
            (Some(group_id), Some(invite)) => DomainEvent::MlsCreated {
                group_id,
                name: string(body, "name"),
                approval: flag(body, "approval"),
                invite,
            },
            _ => DomainEvent::MlsCreateFailed { code: "generic".to_owned() },
        },
        methods::MLS_GROUP_INVITE_CREATE => match (group(), invite(reply)) {
            (Some(group_id), Some(invite)) => DomainEvent::MlsInviteCreated { group_id, invite },
            _ => return Vec::new(),
        },
        methods::MLS_GROUP_INVITE_LIST => DomainEvent::MlsInvitesListed {
            group_id: group().unwrap_or_default(),
            invites: array(reply, "invites").iter().filter_map(invite).collect(),
        },
        methods::MLS_GROUP_INVITE_REVOKE => DomainEvent::MlsInviteRevoked {
            group_id: group().unwrap_or_default(),
            invite_id: string(body, "invite_id"),
        },
        methods::MLS_GROUP_JOIN => DomainEvent::MlsJoinOutcome {
            attempt: number(body, "attempt"),
            outcome: join_outcome(reply),
        },
        methods::MLS_GROUP_JOIN_PENDING => DomainEvent::MlsJoinPendingListed {
            records: array(reply, "requests").iter().filter_map(join_record).collect(),
        },
        methods::MLS_GROUP_MEMBERS => DomainEvent::MlsMembersPage {
            group_id: group().unwrap_or_default(),
            rows: array(reply, "rows").iter().filter_map(member).collect(),
            cursor: opt_string(reply, "cursor"),
            failed: None,
        },
        methods::MLS_GROUP_HISTORY => {
            let direction = match (
                body.get("before_log_seq").and_then(Value::as_u64),
                body.get("after_log_seq").and_then(Value::as_u64),
            ) {
                (Some(ls), _) => HistoryDirection::Before(ls),
                (None, Some(ls)) => HistoryDirection::After(ls),
                (None, None) => HistoryDirection::Newest,
            };
            DomainEvent::MlsHistoryPage {
                group_id: group().unwrap_or_default(),
                direction,
                posts: array(reply, "posts").iter().filter_map(post).collect(),
                floor_log_seq: reply.get("floor_log_seq").and_then(Value::as_u64),
                last_log_seq: number(reply, "last_log_seq"),
                more: flag(reply, "more"),
            }
        }
        methods::MLS_GROUP_SEND => DomainEvent::MlsSendOutcome {
            group_id: group().unwrap_or_default(),
            local_id: string(body, "local_id"),
            post_id: opt_string(reply, "post_id"),
            state: send_state(
                str_of(reply, "state").unwrap_or_default(),
                reply.get("params").unwrap_or(&Value::Null),
                None,
            ),
        },
        methods::MLS_GROUP_MESSAGE_DELETE => DomainEvent::MlsDeleteOutcome {
            group_id: group().unwrap_or_default(),
            post_id: string(body, "post_id"),
            result: match str_of(reply, "state").unwrap_or_default() {
                "queued" => DeleteResult::Queued,
                "delayed_privacy" => DeleteResult::Delayed,
                _ => DeleteResult::Committed { log_seq: number(reply, "log_seq") },
            },
        },
        methods::MLS_GROUP_LEAVE => DomainEvent::MlsLeft {
            group_id: group().unwrap_or_default(),
            result: LeaveResult::Committed,
        },
        methods::MLS_GROUP_REFRESH => {
            let id = group().unwrap_or_default();
            let last = array(reply, "groups")
                .iter()
                .find(|g| str_of(g, "group_id") == Some(id.as_str()))
                .map_or(0, |g| number(g, "last_log_seq"));
            DomainEvent::MlsRefreshed { group_id: id, last_log_seq: last }
        }
        methods::MLS_GROUP_MARK_READ => DomainEvent::MlsMarkedRead {
            group_id: group().unwrap_or_default(),
            read_log_seq: number(reply, "read_log_seq"),
            unread: small(reply, "unread"),
            unread_mentions: small(reply, "unread_mentions"),
        },
        _ => return Vec::new(),
    };
    vec![event]
}

/// The events of an error frame that have their own shape (a join that
/// failed, a send that was not sent, a page that failed, a refused leave or
/// delete, a refused create); `None` for a plain `MlsFailed`.
fn error_outcome(method: &str, body: &Value, error: &BusError) -> Option<DomainEvent> {
    let code = error_code(error);
    let group_id = opt_string(body, "group_id").unwrap_or_default();
    Some(match method {
        methods::MLS_GROUP_JOIN => DomainEvent::MlsJoinOutcome {
            attempt: number(body, "attempt"),
            outcome: JoinOutcome::Failed { code },
        },
        methods::MLS_GROUP_CREATE => DomainEvent::MlsCreateFailed { code },
        methods::MLS_GROUP_MEMBERS => DomainEvent::MlsMembersPage {
            group_id,
            rows: Vec::new(),
            cursor: None,
            failed: Some(code),
        },
        methods::MLS_GROUP_SEND => DomainEvent::MlsSendOutcome {
            group_id,
            local_id: string(body, "local_id"),
            post_id: None,
            state: SendState::NotSent(NotSentReason::from_code(&code, &error_details(error))),
        },
        methods::MLS_GROUP_MESSAGE_DELETE => DomainEvent::MlsDeleteOutcome {
            group_id,
            post_id: string(body, "post_id"),
            result: DeleteResult::Failed { code },
        },
        methods::MLS_GROUP_LEAVE => DomainEvent::MlsLeft {
            group_id,
            result: if code == "owner_protected" {
                LeaveResult::OwnerProtected
            } else {
                LeaveResult::Failed { code }
            },
        },
        _ => return None,
    })
}

/// The events of a refusal on a live source before the pin batch (W): the
/// call is not sent, and the typed unavailable outcome names what it was
/// for. The link and the text of the call are not carried.
pub fn unavailable(method: &str, body: &Value) -> DomainEvent {
    DomainEvent::MlsUnavailable {
        method: method.to_owned(),
        group_id: opt_string(body, "group_id"),
        attempt: body.get("attempt").and_then(Value::as_u64),
        local_id: opt_string(body, "local_id"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_group_event_of_a_kind_m2_skips_is_dropped() {
        for kind in ["pending_join", "catch_up", "warning", "void"] {
            let frame = json!({ "seq": 1, "group_id": "g", "kind": kind, "secret": "x" });
            assert!(group_event(&frame).is_none(), "{kind}");
        }
    }

    #[test]
    fn a_join_reply_reads_every_state() {
        assert_eq!(
            join_outcome(&json!({ "state": "joined", "group_id": "g" })),
            JoinOutcome::Joined { group_id: "g".to_owned() }
        );
        assert!(matches!(
            join_outcome(&json!({ "state": "refused", "reason": "banana" })),
            JoinOutcome::Refused(JoinRefusal::Unknown(_))
        ));
        assert!(matches!(
            join_outcome(&json!({ "state": "pending", "request_id": "r", "reason": "awaiting_admin", "expires_at": 5 })),
            JoinOutcome::Pending { .. }
        ));
    }
}
