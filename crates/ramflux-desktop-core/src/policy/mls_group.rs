// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for MLS groups.
//!
//! Joining, creating, creating and revoking invites, sending, retrying,
//! deleting and leaving are the attended human's alone; so are the reads,
//! which an agent has no use for here. A join is refused while a call is in
//! flight for the account, and its link must pass the local check (S2): the
//! source is never called for a link that does not. A group that is read
//! only (removed, banned or left) takes no send and no refresh (S13). Retry
//! is a send with a new `local_id` that names the bubble it replaces, and
//! only for a bubble that was not sent. Nothing here sends anything again:
//! each intent is one call.

use serde_json::{Value, json};

use super::{Decision, IntentError, Policy};
use crate::intent::{IntentKind, methods};
use crate::model::mls_group::{
    self as mls, CatchUpState, HistoryDirection, MlsRole, OpaqueId, PostState, RequestRowState,
    SettingsField, WarningCode,
};

/// Rows asked of a list page.
const LIST_LIMIT: u32 = 50;
/// Rows asked of a members page.
const MEMBERS_LIMIT: u32 = 200;
/// Posts asked of the newest page and of an older one.
const HISTORY_LIMIT: u32 = 50;
/// Posts asked of a gap fill.
const GAP_LIMIT: u32 = 200;
/// Join requests asked of a page.
const REQUESTS_LIMIT: u32 = 200;

/// Invite durations offered, in seconds (1 and 7 days; the host caps an invite at 7 days).
pub const INVITE_TTLS: [u64; 2] = [86_400, 604_800];
/// Invite use limits offered.
pub const INVITE_USES: [u32; 3] = [1, 10, 100];

/// Decides MLS group kinds; `None` for kinds of other areas.
pub(super) fn decide(p: &Policy<'_>, kind: &IntentKind) -> Option<Result<Decision, IntentError>> {
    let result = match kind {
        IntentKind::MlsGroupList { cursor } => ready(p).map(|()| {
            call(
                methods::MLS_GROUP_LIST,
                json!({ "cursor": cursor, "limit": LIST_LIMIT }),
            )
        }),
        // The read carries the group's event counter from now: what an event
        // applies after it wins over what the read says (common 2.7).
        IntentKind::MlsGroupInfo { group_id } => known(p, group_id).map(|()| {
            call(
                methods::MLS_GROUP_INFO,
                json!({ "group_id": group_id, "issued": p.store.mls_event_gen(group_id) }),
            )
        }),
        IntentKind::MlsGroupMembers { group_id, cursor } => known(p, group_id).map(|()| {
            call(
                methods::MLS_GROUP_MEMBERS,
                json!({ "group_id": group_id, "cursor": cursor, "limit": MEMBERS_LIMIT }),
            )
        }),
        IntentKind::MlsGroupHistory { group_id, direction } => known(p, group_id).map(|()| {
            let body = match direction {
                HistoryDirection::Newest => {
                    json!({ "group_id": group_id, "limit": HISTORY_LIMIT })
                }
                HistoryDirection::Before(log_seq) => {
                    json!({ "group_id": group_id, "before_log_seq": log_seq, "limit": HISTORY_LIMIT })
                }
                HistoryDirection::After(log_seq) => {
                    json!({ "group_id": group_id, "after_log_seq": log_seq, "limit": GAP_LIMIT })
                }
            };
            call(methods::MLS_GROUP_HISTORY, body)
        }),
        IntentKind::MlsGroupRefresh { group_id } => known(p, group_id).and_then(|()| {
            if p.store.mls_read_only(group_id).is_some() {
                return Err(IntentError::MlsReadOnly);
            }
            if !p.store.mls_refresh_due(group_id, p.now) {
                return Err(IntentError::MlsRefreshCoalesced);
            }
            Ok(call(methods::MLS_GROUP_REFRESH, json!({ "group_id": group_id })))
        }),
        IntentKind::MlsGroupMarkRead { group_id, log_seq } => known(p, group_id).and_then(|()| {
            if p.store.mls_read_only(group_id).is_some() {
                return Err(IntentError::MlsReadOnly);
            }
            let read = p.store.mls_group(group_id).map_or(0, |g| g.read_log_seq);
            if *log_seq <= read {
                return Err(IntentError::MlsAlreadyRead);
            }
            Ok(call(
                methods::MLS_GROUP_MARK_READ,
                json!({ "group_id": group_id, "log_seq": log_seq }),
            ))
        }),
        IntentKind::MlsGroupCreate { name, approval } => {
            p.human_only("create a group").and_then(|()| ready(p)).and_then(|()| {
                if !mls::check_name(name) {
                    return Err(IntentError::MlsNameInvalid);
                }
                Ok(call(
                    methods::MLS_GROUP_CREATE,
                    json!({ "name": name.trim(), "tier": 1000, "approval": approval }),
                ))
            })
        }
        // Any member may create invites (r4 item 9).
        IntentKind::MlsInviteCreate { group_id, max_uses, ttl_s } => {
            p.human_only("create an invite").and_then(|()| writable(p, group_id)).and_then(|()| {
                if !INVITE_USES.contains(max_uses) || !INVITE_TTLS.contains(ttl_s) {
                    return Err(IntentError::MlsInviteInvalid);
                }
                Ok(call(
                    methods::MLS_GROUP_INVITE_CREATE,
                    json!({ "group_id": group_id, "max_uses": max_uses, "ttl_s": ttl_s }),
                ))
            })
        }
        IntentKind::MlsInviteList { group_id } => {
            p.human_only("list invites").and_then(|()| known(p, group_id)).map(|()| {
                call(methods::MLS_GROUP_INVITE_LIST, json!({ "group_id": group_id }))
            })
        }
        IntentKind::MlsInviteRevoke { group_id, invite_id } => {
            p.human_only("revoke an invite").and_then(|()| writable(p, group_id)).map(|()| {
                call(
                    methods::MLS_GROUP_INVITE_REVOKE,
                    json!({ "group_id": group_id, "invite_id": invite_id }),
                )
            })
        }
        IntentKind::MlsGroupJoin { link, attempt } => {
            p.human_only("join a group").and_then(|()| ready(p)).and_then(|()| {
                // The local check runs before any call (S2).
                if mls::check_link(link.expose()).as_ref() != Some(link) {
                    return Err(IntentError::MlsLinkInvalid);
                }
                // One call in flight per account (3.1 rule 1).
                if p.store.mls_join().in_flight.is_some() {
                    return Err(IntentError::MlsJoinBusy);
                }
                if *attempt != p.store.mls_next_attempt() {
                    return Err(IntentError::MlsJoinAttempt);
                }
                Ok(call(
                    methods::MLS_GROUP_JOIN,
                    json!({ "link": link.expose(), "attempt": attempt }),
                ))
            })
        }
        IntentKind::MlsJoinPending => {
            ready(p).map(|()| call(methods::MLS_GROUP_JOIN_PENDING, json!({})))
        }
        IntentKind::MlsGroupSend { group_id, text, reply_to, mentions, local_id, retry_of } => {
            p.human_only("send in a group").and_then(|()| writable(p, group_id)).and_then(|()| {
                if p.store.mls_rate_locked_until(group_id, p.now).is_some() {
                    return Err(IntentError::MlsRateLocked);
                }
                if text.trim().is_empty()
                    || local_id.is_empty()
                    || mentions.len() > mls::MAX_MENTIONS
                {
                    return Err(IntentError::MlsPostInvalid);
                }
                if let Some(old) = retry_of {
                    let unsent = p
                        .store
                        .mls_not_sent(group_id)
                        .iter()
                        .any(|item| &item.local_id == old);
                    if !unsent {
                        return Err(IntentError::MlsNothingToRetry);
                    }
                }
                Ok(call(
                    methods::MLS_GROUP_SEND,
                    json!({
                        "group_id": group_id,
                        "text": text,
                        "reply_to": reply_to,
                        "mentions": mentions,
                        "local_id": local_id,
                        "retry_of": retry_of,
                    }),
                ))
            })
        }
        IntentKind::MlsGroupDelete { group_id, post_id } => {
            p.human_only("delete a post").and_then(|()| writable(p, group_id)).and_then(|()| {
                let my = p.store.mls_my_anchor();
                let own = p.store.mls_timeline(group_id).is_some_and(|tl| {
                    tl.posted.values().any(|post| {
                        &post.post_id == post_id
                            && post.state != PostState::Deleted
                            && post.author.as_ref().is_some_and(|a| {
                                a.is_me || my.is_some_and(|m| !m.is_empty() && m == a.anchor)
                            })
                    })
                });
                if !own {
                    return Err(IntentError::MlsNotOwnPost);
                }
                Ok(call(
                    methods::MLS_GROUP_MESSAGE_DELETE,
                    json!({ "group_id": group_id, "post_id": post_id }),
                ))
            })
        }
        IntentKind::MlsGroupLeave { group_id } => {
            p.human_only("leave a group").and_then(|()| writable(p, group_id)).map(|()| {
                call(methods::MLS_GROUP_LEAVE, json!({ "group_id": group_id }))
            })
        }
        other => return decide_admin(p, other),
    };
    Some(result)
}

fn call(method: &'static str, body: Value) -> Decision {
    Decision::Call(method, body)
}

/// The account is open for MLS calls.
fn ready(p: &Policy<'_>) -> Result<(), IntentError> {
    if p.store.mls_locked() { Err(IntentError::MlsLocked) } else { Ok(()) }
}

/// The group is one this account knows (listed, or a chat still kept).
fn known(p: &Policy<'_>, group_id: &str) -> Result<(), IntentError> {
    ready(p)?;
    if p.store.mls_group(group_id).is_some()
        || p.store.mls_timeline(group_id).is_some()
        || p.store.context(&mls::context_id(group_id)).is_some()
    {
        Ok(())
    } else {
        Err(IntentError::MlsGroupUnknown)
    }
}

/// The group is known and takes calls from this account (S13).
fn writable(p: &Policy<'_>, group_id: &str) -> Result<(), IntentError> {
    known(p, group_id)?;
    if p.store.mls_read_only(group_id).is_some() { Err(IntentError::MlsReadOnly) } else { Ok(()) }
}

// ---- M3: admin and edge states -------------------------------------------

/// Adds the account epoch and a new action id to a call body: every M3 call
/// carries them and its outcome echoes them. They never go on the wire.
fn stamped(p: &Policy<'_>, mut body: Value) -> Value {
    body["epoch"] = json!(p.store.mls_epoch());
    body["action_id"] = json!(p.store.mls_next_action_id());
    body
}

/// The group takes admin calls from this account: it is writable, the role
/// allows it (`owner_only` for role changes), and, for a commit, no other
/// admin action holds the group and commits are not paced.
fn admin_gate(
    p: &Policy<'_>,
    group_id: &str,
    owner_only: bool,
    commit: bool,
) -> Result<(), IntentError> {
    writable(p, group_id)?;
    match p.store.mls_my_role(group_id) {
        Some(MlsRole::Owner) => {}
        Some(MlsRole::Admin) if !owner_only => {}
        _ => return Err(IntentError::MlsNotAllowed),
    }
    if commit {
        if p.store.mls_admin_busy(group_id) {
            return Err(IntentError::MlsAdminBusy);
        }
        if p.store.mls_paced_until(group_id, p.now).is_some() {
            return Err(IntentError::MlsPaced);
        }
    }
    Ok(())
}

/// The group needs approval (the requests exist only then).
fn approval_on(p: &Policy<'_>, group_id: &str) -> Result<(), IntentError> {
    if p.store.mls_group(group_id).is_some_and(|g| g.approval) {
        Ok(())
    } else {
        Err(IntentError::MlsApprovalOff)
    }
}

/// An idle request row that has not expired.
fn open_request(p: &Policy<'_>, group_id: &str, id: &OpaqueId) -> Result<(), IntentError> {
    let row = p
        .store
        .mls_requests(group_id)
        .and_then(|pager| pager.row(id))
        .ok_or(IntentError::MlsTargetUnknown)?;
    if row.state != RequestRowState::Idle {
        return Err(IntentError::MlsInFlight);
    }
    if row.request.expires_at.is_some_and(|at| at.0 <= p.now.0) {
        return Err(IntentError::MlsTargetUnknown);
    }
    Ok(())
}

/// The loaded member row of `anchor`, when it is not this account and not
/// the owner.
fn member_target<'a>(
    p: &'a Policy<'_>,
    group_id: &str,
    anchor: &OpaqueId,
) -> Result<&'a mls::MemberRow, IntentError> {
    let row = p
        .store
        .mls_members(group_id)
        .and_then(|pages| pages.rows.get(anchor.expose()))
        .ok_or(IntentError::MlsTargetUnknown)?;
    if row.is_me || row.role == MlsRole::Owner {
        return Err(IntentError::MlsNotAllowed);
    }
    Ok(row)
}

fn decide_admin(p: &Policy<'_>, kind: &IntentKind) -> Option<Result<Decision, IntentError>> {
    let result = match kind {
        IntentKind::MlsRequestsList { group_id, cursor } => {
            p.human_only("list join requests").and_then(|()| known(p, group_id)).and_then(|()| {
                if !p.store.mls_is_admin(group_id) {
                    return Err(IntentError::MlsNotAllowed);
                }
                approval_on(p, group_id)?;
                let pager = p.store.mls_requests(group_id);
                let first = cursor.is_none();
                // One call per page: the same page is not asked twice, and
                // a later page is the one the cursor names.
                if first && pager.is_some_and(|r| r.loading) {
                    return Err(IntentError::MlsInFlight);
                }
                if !first && (pager.is_none_or(|r| r.loading_more || r.cursor != *cursor)) {
                    return Err(IntentError::MlsInFlight);
                }
                Ok(call(
                    methods::MLS_GROUP_REQUESTS_LIST,
                    json!({
                        "group_id": group_id,
                        "cursor": cursor,
                        "limit": REQUESTS_LIMIT,
                        "page_gen": p.store.mls_requests_gen(group_id, first),
                    }),
                ))
            })
        }
        IntentKind::MlsRequestApprove { group_id, request_id } => p
            .human_only("approve a join request")
            .and_then(|()| admin_gate(p, group_id, false, true))
            .and_then(|()| approval_on(p, group_id))
            .and_then(|()| open_request(p, group_id, request_id))
            .map(|()| {
                call(
                    methods::MLS_GROUP_REQUESTS_APPROVE,
                    stamped(p, json!({ "group_id": group_id, "request_id": request_id.expose() })),
                )
            }),
        IntentKind::MlsRequestDecline { group_id, request_id } => p
            .human_only("decline a join request")
            .and_then(|()| admin_gate(p, group_id, false, false))
            .and_then(|()| approval_on(p, group_id))
            .and_then(|()| open_request(p, group_id, request_id))
            .and_then(|()| {
                if p.store.mls_declining(group_id, request_id) {
                    return Err(IntentError::MlsInFlight);
                }
                Ok(call(
                    methods::MLS_GROUP_REQUESTS_DECLINE,
                    json!({
                        "group_id": group_id,
                        "request_id": request_id.expose(),
                        "epoch": p.store.mls_epoch(),
                    }),
                ))
            }),
        IntentKind::MlsRoleSet { group_id, anchor, admin } => {
            p.human_only("change a role")
                .and_then(|()| admin_gate(p, group_id, true, true))
                .and_then(|()| member_target(p, group_id, anchor).map(|row| row.role.clone()))
                .and_then(|role| {
                    // Promote a member, or demote an admin; nothing else.
                    let valid =
                        matches!((admin, role), (true, MlsRole::Member) | (false, MlsRole::Admin));
                    if !valid {
                        return Err(IntentError::MlsNotAllowed);
                    }
                    Ok(call(
                        methods::MLS_GROUP_ROLE_SET,
                        stamped(
                            p,
                            json!({
                                "group_id": group_id,
                                "anchor": anchor.expose(),
                                "role": if *admin { "admin" } else { "member" },
                            }),
                        ),
                    ))
                })
        }
        IntentKind::MlsMemberRemove { group_id, anchor }
        | IntentKind::MlsMemberBan { group_id, anchor } => {
            let (method, what) = if matches!(kind, IntentKind::MlsMemberRemove { .. }) {
                (methods::MLS_GROUP_MEMBER_REMOVE, "remove a member")
            } else {
                (methods::MLS_GROUP_MEMBER_BAN, "ban a member")
            };
            p.human_only(what)
                .and_then(|()| admin_gate(p, group_id, false, true))
                .and_then(|()| member_target(p, group_id, anchor).map(|row| row.role.clone()))
                .and_then(|role| {
                    // An admin acts on plain members only.
                    if p.store.mls_my_role(group_id) == Some(&MlsRole::Admin)
                        && role != MlsRole::Member
                    {
                        return Err(IntentError::MlsNotAllowed);
                    }
                    Ok(call(
                        method,
                        stamped(p, json!({ "group_id": group_id, "anchor": anchor.expose() })),
                    ))
                })
        }
        IntentKind::MlsSettingsSet { group_id, field } => {
            p.human_only("change group settings")
                .and_then(|()| admin_gate(p, group_id, false, true))
                .and_then(|()| {
                    let summary =
                        p.store.mls_group(group_id).ok_or(IntentError::MlsGroupUnknown)?;
                    // One field per call.
                    let body = match field {
                        SettingsField::Name(name) => {
                            if !mls::name_change_valid(name, &summary.name) {
                                return Err(IntentError::MlsSettingInvalid);
                            }
                            json!({ "group_id": group_id, "name": name.trim() })
                        }
                        SettingsField::Approval(on) => {
                            if *on == summary.approval {
                                return Err(IntentError::MlsSettingInvalid);
                            }
                            json!({ "group_id": group_id, "approval": on })
                        }
                    };
                    Ok(call(methods::MLS_GROUP_SETTINGS_SET, stamped(p, body)))
                })
        }
        IntentKind::MlsPushSet { group_id, push } => p
            .human_only("change notifications")
            .and_then(|()| writable(p, group_id))
            .and_then(|()| {
                let token = push.token().ok_or(IntentError::MlsSettingInvalid)?;
                if p.store.mls_push_saving(group_id).is_some() {
                    return Err(IntentError::MlsInFlight);
                }
                Ok(call(
                    methods::MLS_GROUP_PUSH_SET,
                    json!({
                        "group_id": group_id,
                        "push": token,
                        "epoch": p.store.mls_epoch(),
                    }),
                ))
            }),
        IntentKind::MlsCatchUpSkip { group_id } => {
            p.human_only("skip the catch-up").and_then(|()| writable(p, group_id)).and_then(|()| {
                let record = p
                    .store
                    .mls_catch_up(group_id)
                    .filter(|r| r.state == CatchUpState::Running)
                    .ok_or(IntentError::MlsNotCatchingUp)?;
                if record.skipping.is_some() {
                    return Err(IntentError::MlsInFlight);
                }
                Ok(call(
                    methods::MLS_GROUP_CATCH_UP,
                    json!({
                        "group_id": group_id,
                        "skip": true,
                        "epoch": p.store.mls_epoch(),
                        "from_seq": record.seq,
                    }),
                ))
            })
        }
        IntentKind::MlsVoid { group_id, log_seq } => {
            p.human_only("undo a change")
                .and_then(|()| admin_gate(p, group_id, false, true))
                .and_then(|()| {
                    // The head commit, or the one `head_commit_rejected` names.
                    let named = p.store.mls_warnings(group_id).into_iter().any(|w| {
                        matches!(w, WarningCode::HeadCommitRejected { log_seq: l } if l == log_seq)
                    });
                    let head = p.store.mls_head_commit(group_id) == Some(*log_seq);
                    if !(head || named) || p.store.mls_voided(group_id).contains(log_seq) {
                        return Err(IntentError::MlsVoidTarget);
                    }
                    Ok(call(
                        methods::MLS_GROUP_VOID,
                        stamped(p, json!({ "group_id": group_id, "log_seq": log_seq })),
                    ))
                })
        }
        _ => return None,
    };
    Some(result)
}
