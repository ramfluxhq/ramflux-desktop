// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Exporting the credentials of an AI participant (DESK-AGENT-01; see
//! `docs/contract.md`, "AI participants").
//!
//! `account.export_agent_credentials {account, path}` runs for the owner's
//! account (the frame's `account_id`) and only on a connection that
//! attended it ([`crate::bus::attended`]). The body carries exactly the two
//! fields the daemon takes (it refuses unknown ones): the AI participant
//! and the location the owner chose. The answer holds the same two and
//! `exported: true`; the bundle itself is written by the daemon, readable
//! by its owner only, and never passes through the desktop.

use ramflux_desktop_core::methods;
use serde_json::{Value, json};

use super::{Plan, call, refuse_invalid, text};
use crate::bus::BusRequest;

/// The `sdk_api` label of the request.
pub const API: &str = "account";

/// Plans an AI participant effect.
pub fn agent_effect(method: &str, body: &Value) -> Plan {
    match method {
        methods::AGENT_EXPORT_CREDENTIALS => {
            let (owner, account, path) =
                (text(body, "owner_id"), text(body, "account"), text(body, "path"));
            if owner.is_empty() || account.is_empty() || path.is_empty() {
                return refuse_invalid("the export names an owner, an AI participant and a place");
            }
            call(BusRequest::new(
                Some(owner),
                API,
                "account.export_agent_credentials",
                json!({ "account": account, "path": path }),
            ))
        }
        _ => refuse_invalid("no daemon counterpart"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_export_runs_for_the_owner_with_only_the_two_fields_the_daemon_takes() {
        let body = json!({
            "owner_id": "acct-main", "account": "acct-bot", "path": "/Users/a/bot.bundle",
            "proposed_by": "Human", "confirmed_by": null,
        });
        let Plan::Call { request, .. } = agent_effect(methods::AGENT_EXPORT_CREDENTIALS, &body)
        else {
            panic!("a call");
        };
        assert_eq!(request.account.as_deref(), Some("acct-main"));
        assert_eq!(request.method, "account.export_agent_credentials");
        assert_eq!(request.body, json!({ "account": "acct-bot", "path": "/Users/a/bot.bundle" }));
        assert!(request.secrets.is_empty());
    }

    #[test]
    fn an_export_that_names_nothing_is_not_sent() {
        let body = json!({ "owner_id": "acct-main", "account": "", "path": "/x" });
        assert!(matches!(
            agent_effect(methods::AGENT_EXPORT_CREDENTIALS, &body),
            Plan::Refuse { .. }
        ));
    }
}
