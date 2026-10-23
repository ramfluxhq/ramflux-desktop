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
    /// `object.directory`.
    Objects,
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
            Self::Objects => ("object", "object.directory", "objects"),
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
    /// `message.list` of one direct context.
    Messages(ContextId),
    /// `message.receive` for one direct context (pull from the node), read
    /// back through [`Fetch::Messages`].
    Receive(ContextId),
    /// `group.receive` for one group conversation (pull from the node);
    /// answers with the conversation's stored messages.
    GroupReceive {
        /// The conversation (the context).
        context: ContextId,
        /// The group it belongs to, as the conversation directory links it.
        group: String,
    },
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
            Self::DaemonStatus => BusRequest::new(None, "daemon", "daemon.status", empty),
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
            Self::Messages(context) => BusRequest::new(
                account,
                "message",
                "message.list",
                json!({ "conversation_id": context }),
            ),
            // Attachments are fetched with their message: the daemon lists
            // an object (and can open it) only once it is stored locally.
            Self::Receive(context) => BusRequest::new(
                account,
                "message",
                "message.receive",
                json!({
                    "limit": 100,
                    "conversation_id": context,
                    "auto_fetch_attachments": true,
                }),
            ),
            Self::GroupReceive { context, group } => BusRequest::new(
                account,
                "group",
                "group.receive",
                json!({ "group_id": group, "conversation_id": context, "limit": 100 }),
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
            Self::first(Paged::Objects),
        ]
    }

    /// The lists an account poll refreshes, each a first page.
    pub fn account_poll() -> Vec<Self> {
        [Paged::Identities, Paged::Groups, Paged::Conversations, Paged::Approvals, Paged::Objects]
            .into_iter()
            .map(Self::first)
            .chain([Self::ContactList])
            .collect()
    }
}
