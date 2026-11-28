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
use crate::model::mls_group::{self as mls, HistoryDirection, PostState};

/// Rows asked of a list page.
const LIST_LIMIT: u32 = 50;
/// Rows asked of a members page.
const MEMBERS_LIMIT: u32 = 200;
/// Posts asked of the newest page and of an older one.
const HISTORY_LIMIT: u32 = 50;
/// Posts asked of a gap fill.
const GAP_LIMIT: u32 = 200;

/// Invite durations offered, in seconds (1, 7 and 30 days).
pub const INVITE_TTLS: [u64; 3] = [86_400, 604_800, 2_592_000];
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
        IntentKind::MlsGroupInfo { group_id } => {
            known(p, group_id).map(|()| call(methods::MLS_GROUP_INFO, json!({ "group_id": group_id })))
        }
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
        _ => return None,
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
