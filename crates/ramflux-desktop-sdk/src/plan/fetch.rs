// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Reads: every daemon list or status the source pulls, as a request.

use ramflux_desktop_core::{AccountId, ContextId};
use serde_json::json;

use crate::bus::BusRequest;

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
    /// `conversation.list`.
    ConversationList,
    /// `group.list`.
    GroupList,
    /// `contact.list`.
    ContactList,
    /// `mcp.approval.list`.
    Approvals,
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
    /// `group.receive` for one group (pull from the node); answers with the
    /// group's stored messages.
    GroupReceive(ContextId),
}

impl Fetch {
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
            Self::ConversationList => {
                BusRequest::new(account, "message", "conversation.list", empty)
            }
            Self::GroupList => BusRequest::new(account, "group", "group.list", empty),
            Self::ContactList => BusRequest::new(account, "contact", "contact.list", empty),
            Self::Approvals => BusRequest::new(account, "mcp", "mcp.approval.list", empty),
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
            Self::Receive(context) => BusRequest::new(
                account,
                "message",
                "message.receive",
                json!({
                    "limit": 100,
                    "conversation_id": context,
                    "auto_fetch_attachments": false,
                }),
            ),
            // Assumption: a group's conversation id is its group id (the
            // daemon does not link them in `group.list`).
            Self::GroupReceive(context) => BusRequest::new(
                account,
                "group",
                "group.receive",
                json!({ "group_id": context, "conversation_id": context, "limit": 100 }),
            ),
        }
    }

    /// The reads of the daemon scope's snapshot, in translation order.
    pub fn daemon_snapshot() -> Vec<Self> {
        vec![Self::DaemonStatus, Self::NodeList, Self::AccountList]
    }

    /// The reads of an account's snapshot, in translation order.
    pub fn account_snapshot(account: &AccountId) -> Vec<Self> {
        vec![
            Self::DaemonStatus,
            Self::NodeList,
            Self::AccountStatus(account.clone()),
            Self::DeviceList,
            Self::ConversationList,
            Self::GroupList,
            Self::ContactList,
            Self::Approvals,
            Self::Grants,
            Self::StandingApprovals,
            Self::McpTools,
        ]
    }
}
