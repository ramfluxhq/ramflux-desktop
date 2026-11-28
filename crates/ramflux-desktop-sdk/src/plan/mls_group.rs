// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! MLS groups: contract methods to wire requests (method table r4).
//!
//! | contract | wire | request |
//! |---|---|---|
//! | `mls_group.list` | `group.list` | `{cursor?, limit}` |
//! | `mls_group.info` | `group.info` | `{group_id}` |
//! | `mls_group.create` | `group.create` | `{name, tier, approval}` |
//! | `mls_group.invite.create` | `group.invite.create` | `{group_id, max_uses, ttl_s}` |
//! | `mls_group.invite.list` | `group.invite.list` | `{group_id}` |
//! | `mls_group.invite.revoke` | `group.invite.revoke` | `{group_id, invite_id}` |
//! | `mls_group.join` | `group.join` | `{link}` (the attempt stays here) |
//! | `mls_group.join.pending` | `group.join.pending` | `{}` |
//! | `mls_group.members` | `group.members` | `{group_id, cursor?, limit}` |
//! | `mls_group.history` | `group.history` | `{group_id, after_log_seq? or before_log_seq?, limit}` |
//! | `mls_group.send` | `group.send` | `{group_id, text, reply_to?, mentions}` |
//! | `mls_group.refresh` | `group.refresh` | `{group_id}` |
//! | `mls_group.mark_read` | `group.mark_read` | `{group_id, log_seq}` |
//! | `mls_group.message.delete` | `group.message.delete` | `{group_id, post_id}` |
//! | `mls_group.leave` | `group.leave` | `{group_id}` |
//!
//! Five of these wire names exist at the current pin with the old sender-key
//! shapes (`group.history`, `group.leave`, `group.members`, `group.send` and
//! the renamed `group.read`), so the bridge refuses every `mls_group.*`
//! method on a live source until the pin batch (W); these plans are written
//! now and are unused on live until then. The link of a join is passed
//! verbatim and never logged.

use ramflux_desktop_core::{SecretInput, methods};
use serde_json::{Value, json};

use super::{Plan, call, refuse_invalid, text};
use crate::bus::BusRequest;

/// Plans one MLS group method.
pub(super) fn plan(acct: Option<&str>, method: &str, body: &Value) -> Plan {
    let group = || text(body, "group_id");
    let request =
        |wire: &'static str, body: Value| call(BusRequest::new(acct, "group", wire, body));
    match method {
        methods::MLS_GROUP_LIST => {
            request("group.list", json!({ "cursor": body["cursor"], "limit": body["limit"] }))
        }
        methods::MLS_GROUP_INFO => request("group.info", json!({ "group_id": group() })),
        methods::MLS_GROUP_CREATE => request(
            "group.create",
            json!({ "name": text(body, "name"), "tier": body["tier"], "approval": body["approval"] }),
        ),
        methods::MLS_GROUP_INVITE_CREATE => request(
            "group.invite.create",
            json!({ "group_id": group(), "max_uses": body["max_uses"], "ttl_s": body["ttl_s"] }),
        ),
        methods::MLS_GROUP_INVITE_LIST => {
            request("group.invite.list", json!({ "group_id": group() }))
        }
        methods::MLS_GROUP_INVITE_REVOKE => request(
            "group.invite.revoke",
            json!({ "group_id": group(), "invite_id": text(body, "invite_id") }),
        ),
        // The link only: the attempt number never goes on the wire.
        // The link only (the attempt number never goes on the wire). It
        // travels as a secret field, so no `Debug` of the request prints it.
        methods::MLS_GROUP_JOIN => call(
            BusRequest::new(acct, "group", "group.join", json!({}))
                .with_secret("link", SecretInput::new(text(body, "link"))),
        ),
        methods::MLS_GROUP_JOIN_PENDING => request("group.join.pending", json!({})),
        methods::MLS_GROUP_MEMBERS => request(
            "group.members",
            json!({ "group_id": group(), "cursor": body["cursor"], "limit": body["limit"] }),
        ),
        methods::MLS_GROUP_HISTORY => {
            let mut wire = json!({ "group_id": group(), "limit": body["limit"] });
            for field in ["after_log_seq", "before_log_seq"] {
                if let Some(value) = body.get(field).filter(|v| !v.is_null()) {
                    wire[field] = value.clone();
                }
            }
            request("group.history", wire)
        }
        methods::MLS_GROUP_SEND => {
            let mut wire = json!({
                "group_id": group(),
                "text": text(body, "text"),
                "mentions": body["mentions"],
            });
            if let Some(reply) = body.get("reply_to").filter(|v| !v.is_null()) {
                wire["reply_to"] = reply.clone();
            }
            request("group.send", wire)
        }
        methods::MLS_GROUP_REFRESH => request("group.refresh", json!({ "group_id": group() })),
        methods::MLS_GROUP_MARK_READ => {
            request("group.mark_read", json!({ "group_id": group(), "log_seq": body["log_seq"] }))
        }
        methods::MLS_GROUP_MESSAGE_DELETE => request(
            "group.message.delete",
            json!({ "group_id": group(), "post_id": text(body, "post_id") }),
        ),
        methods::MLS_GROUP_LEAVE => request("group.leave", json!({ "group_id": group() })),
        _ => refuse_invalid("no daemon counterpart"),
    }
}
