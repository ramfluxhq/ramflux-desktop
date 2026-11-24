// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Reads: every daemon list or status the source pulls, as a request.
//!
//! The client directories and the lists that accept paging are read a page
//! at a time ([`Paged`]): the first page without a cursor, each next page
//! with the cursor the previous one returned, until the daemon returns none.
//! The daemon bounds each page by its response budget, so a page may hold
//! fewer rows than asked for and still have a next one.

use ramflux_desktop_core::{AccountId, ContextId};
use serde_json::json;

use crate::bus::BusRequest;

/// Rows asked for per page (the daemon's maximum).
pub const PAGE_LIMIT: u32 = 100;

/// A paged list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Paged {
    /// `identity.directory`.
    Identities,
    /// `conversation.directory`.
    Conversations,
    /// `group.list`.
    Groups,
    /// `mcp.approval.list`.
    Approvals,
    /// `mcp.server.list`.
    McpServers,
}

impl Paged {
    /// The `sdk_api` label, daemon method and the response field holding the rows.
    pub const fn method(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Identities => ("contact", "identity.directory", "identities"),
            Self::Conversations => ("message", "conversation.directory", "conversations"),
            Self::Groups => ("group", "group.list", "groups"),
            Self::Approvals => ("mcp", "mcp.approval.list", "approvals"),
            Self::McpServers => ("mcp", "mcp.server.list", "server_details"),
        }
    }
}

/// One read of daemon state.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fetch {
    /// `daemon.status`.
    DaemonStatus,
    /// `daemon.status` read for the schema gate of one connection epoch: the
    /// answer is never translated, and an answer of another epoch is ignored.
    SchemaStatus(u64),
    /// `node.list`.
    NodeList,
    /// `account.list`.
    AccountList,
    /// `account.status` of one account.
    AccountStatus(AccountId),
    /// `device.list`.
    DeviceList,
    /// One page of a paged list; `None` asks for the first page.
    Page(Paged, Option<String>),
    /// `contact.list`.
    ContactList,
    /// `grant.list`.
    Grants,
    /// `grant.list_standing_approvals`.
    StandingApprovals,
    /// `mcp.tool.list`.
    McpTools,
    /// `message.read` (the read model's newest page) of one conversation,
    /// direct or group.
    Messages(ContextId),
    /// `message.read` of the page older than `before` (the `next_before`
    /// event identity of the last page, as JSON text).
    MessagesBefore(ContextId, String),
    /// `message.read` of the rows changed after journal sequence `after`.
    MessageChanges(ContextId, u64),
    /// `surface.list` of one conversation; `None` asks for the first page.
    Surfaces(ContextId, Option<String>),
    /// `surface.get` of one surface, by the daemon's scoped `surface_ref`.
    Surface(String),
    /// `task.list` of one conversation; `None` asks for the first page.
    Tasks(ContextId, Option<String>),
    /// `task.get` of one task: its conversation and the daemon's task id.
    Task(ContextId, String),
    /// `contact.requests.list`: the pending contact requests and the
    /// account's request setting. Its answer also tells whether the daemon
    /// has contact requests (CONTACT-02).
    ContactRequests,
    /// `call.list`: the account's ringing and active calls and recent call
    /// records. Its answer also tells whether the daemon has the call API.
    Calls,
    /// `account.backup.jobs`: the account's backup and restore jobs (running
    /// and recent). Its answer also tells whether the daemon has the full
    /// backup API.
    BackupJobs,
    /// `group.invitations.list`: groups waiting for this account's answer.
    GroupInvitations,
    /// `username.status`: this account's username, or why the directory
    /// cannot be used.
    UsernameStatus,
}

impl Fetch {
    /// The first page of `paged`.
    pub const fn first(paged: Paged) -> Self {
        Self::Page(paged, None)
    }

    /// The request; `account` is the scope's account (`None` for the daemon).
    pub fn request(&self, account: Option<&str>) -> BusRequest {
        let empty = json!({});
        match self {
            Self::DaemonStatus | Self::SchemaStatus(_) => {
                BusRequest::new(None, "daemon", "daemon.status", empty)
            }
            Self::NodeList => BusRequest::new(None, "node", "node.list", empty),
            Self::AccountList => BusRequest::new(None, "account", "account.list", empty),
            Self::AccountStatus(id) => {
                BusRequest::new(Some(id.as_str()), "account", "account.status", empty)
            }
            Self::DeviceList => BusRequest::new(account, "device", "device.list", empty),
            Self::Page(paged, cursor) => {
                let (api, method, _) = paged.method();
                BusRequest::new(
                    account,
                    api,
                    method,
                    json!({ "limit": PAGE_LIMIT, "cursor": cursor }),
                )
            }
            Self::ContactList => BusRequest::new(account, "contact", "contact.list", empty),
            Self::Grants => BusRequest::new(account, "grant", "grant.list", empty),
            Self::StandingApprovals => {
                BusRequest::new(account, "grant", "grant.list_standing_approvals", empty)
            }
            Self::McpTools => BusRequest::new(account, "mcp", "mcp.tool.list", empty),
            Self::Calls => BusRequest::new(account, "call", "call.list", empty),
            Self::ContactRequests => {
                BusRequest::new(account, "contact", "contact.requests.list", empty)
            }
            Self::BackupJobs => BusRequest::new(account, "account", "account.backup.jobs", empty),
            Self::GroupInvitations => {
                BusRequest::new(account, "group", "group.invitations.list", empty)
            }
            Self::UsernameStatus => BusRequest::new(
                account,
                "username",
                "username.status",
                json!({ "operation_id": null }),
            ),
            Self::Messages(context) => BusRequest::new(
                account,
                "message",
                "message.read",
                json!({ "conversation_id": context, "read_model": true, "limit": PAGE_LIMIT }),
            ),
            Self::MessagesBefore(context, before) => BusRequest::new(
                account,
                "message",
                "message.read",
                json!({
                    "conversation_id": context,
                    "read_model": true,
                    "limit": PAGE_LIMIT,
                    "before_order_key": serde_json::from_str::<serde_json::Value>(before)
                        .unwrap_or_default(),
                }),
            ),
            Self::MessageChanges(context, after) => BusRequest::new(
                account,
                "message",
                "message.read",
                json!({ "conversation_id": context, "after_sequence": after, "limit": PAGE_LIMIT }),
            ),
            Self::Surfaces(context, cursor) => BusRequest::new(
                account,
                "surface",
                "surface.list",
                json!({
                    "conversation_id": context,
                    "limit": PAGE_LIMIT,
                    "cursor": cursor.as_deref().unwrap_or_default(),
                }),
            ),
            Self::Surface(reference) => BusRequest::new(
                account,
                "surface",
                "surface.get",
                json!({ "surface_ref": reference }),
            ),
            Self::Tasks(context, cursor) => BusRequest::new(
                account,
                "task",
                "task.list",
                json!({ "conversation_id": context, "limit": PAGE_LIMIT, "cursor": cursor }),
            ),
            Self::Task(context, task) => BusRequest::new(
                account,
                "task",
                "task.get",
                json!({ "conversation_id": context, "task_id": task }),
            ),
        }
    }

    /// The reads of the daemon scope's snapshot, in translation order.
    pub fn daemon_snapshot() -> Vec<Self> {
        vec![Self::DaemonStatus, Self::NodeList, Self::AccountList]
    }

    /// The reads of an account's snapshot, in translation order:
    /// identities before the conversations and groups that name them.
    pub fn account_snapshot(account: &AccountId) -> Vec<Self> {
        vec![
            Self::DaemonStatus,
            Self::NodeList,
            Self::AccountStatus(account.clone()),
            Self::DeviceList,
            Self::first(Paged::Identities),
            Self::first(Paged::Groups),
            Self::first(Paged::Conversations),
            Self::ContactList,
            Self::first(Paged::Approvals),
            Self::Grants,
            Self::StandingApprovals,
            Self::McpTools,
            Self::first(Paged::McpServers),
            Self::GroupInvitations,
            Self::UsernameStatus,
        ]
    }
}
