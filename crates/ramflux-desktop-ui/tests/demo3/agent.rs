// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The participants without a window, played over their daemons' local
//! bus: the node and the accounts each daemon holds, and the AI assignee
//! (a headless AI account) doing its side of a task: accepting it, asking
//! for a single-use MCP approval, running the tool once approved and
//! finishing with a signed result summary. The product ships no agent
//! binary; these are the calls an agent makes (as in the product's
//! `k3b_task_needs_me_approval` realnet test).

use std::time::{Duration, Instant};

use ramflux_desktop_test::live_daemon::LiveDaemon;
use serde_json::{Value, json};

/// One account a daemon holds.
#[derive(Debug, Clone)]
pub struct Account {
    /// Local account id.
    pub id: String,
    /// Principal.
    pub principal: String,
}

/// A daemon and the account the rehearsal uses on it.
pub struct Bus<'d> {
    pub daemon: &'d LiveDaemon,
    pub account: Option<Account>,
}

impl Bus<'_> {
    fn acct(&self) -> Option<&str> {
        self.account.as_ref().map(|a| a.id.as_str())
    }

    /// One call for this daemon's account.
    pub fn call(&self, api: &str, method: &str, body: &Value) -> Result<Value, String> {
        self.daemon.call(self.acct(), api, method, body)
    }

    /// Adds the node by URL, checks its key and pins it on first use;
    /// returns the node id.
    pub fn node(&self, url: &str, expected_key: &str) -> Result<String, String> {
        let added = self.daemon.call(None, "node", "node.add", &json!({ "url": url }))?;
        let node = added["node"]["node_id"].as_str().ok_or("node.add named no node")?.to_owned();
        let shown = added["key_fingerprint"].as_str().unwrap_or_default();
        if !crate::rehearsal::config::fingerprint_matches(expected_key, shown) {
            return Err(format!("node {node}: fingerprint {shown:?} is not the expected key"));
        }
        let pin = json!({ "node_id": node, "pin_source": "tofu" });
        self.daemon.call(None, "node", "node.confirm", &pin)?;
        Ok(node)
    }

    /// Creates an account on `node` with this root (same root: another
    /// device of the same principal) and a fresh device; returns the
    /// daemon's answer.
    pub fn create(&mut self, node: &str, who: &Who<'_>, secret: &str) -> Result<Value, String> {
        let (mode, kind) =
            if who.agent { ("headless_ai", "headless_ai") } else { ("attended_cli", "human") };
        let body = json!({
            "node_id": node,
            "local_account_id": who.account,
            "principal_id": who.principal,
            "principal_commitment":
                ramflux_sdk::identity_root_public_key_commitment_for_seed(who.principal, who.root),
            "device_id": format!("device-{}", hex(&random(), 16)),
            "target_delivery_id": format!("target-{}", hex(&random(), 16)),
            "client_mode": mode,
            "kind": kind,
            "label": who.label,
            "account_secret": secret,
            "root_seed": who.root.to_vec(),
            "device_seed": random().to_vec(),
        });
        let created = self.daemon.call(None, "account", "account.create", &body)?;
        self.account =
            Some(Account { id: who.account.to_owned(), principal: who.principal.to_owned() });
        Ok(created)
    }

    /// Adds `principal` as a contact (a lookup through the home node).
    pub fn add_contact(&self, principal: &str) -> Result<Value, String> {
        self.call("contact", "contact.add", &json!({ "principal_id": principal }))
    }

    /// The task `task_id` of `conversation` once its version reaches
    /// `version` (or any task of it, when `task_id` is empty, that
    /// `matches`).
    pub fn task_at(
        &self,
        conversation: &str,
        version: u64,
        limit: Duration,
        matches: impl Fn(&Value) -> bool,
    ) -> Result<Value, String> {
        let start = Instant::now();
        loop {
            let listed = self.call(
                "task",
                "task.list",
                &json!({ "conversation_id": conversation, "cursor": "", "limit": 50 }),
            )?;
            let found = listed["tasks"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|t| t["version"].as_u64().is_some_and(|v| v >= version) && matches(t));
            if let Some(task) = found {
                return Ok(task.clone());
            }
            if start.elapsed() > limit {
                return Err(format!("no task at version {version} in {conversation}: {listed}"));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Waits until `conversation` is in this account's directory.
    pub fn conversation(&self, conversation: &str, limit: Duration) -> Result<(), String> {
        let start = Instant::now();
        loop {
            let listed =
                self.call("conversation", "conversation.directory", &json!({ "limit": 100 }))?;
            if listed["conversations"]
                .as_array()
                .is_some_and(|rows| rows.iter().any(|c| c["id"] == conversation))
            {
                return Ok(());
            }
            if start.elapsed() > limit {
                return Err(format!("{conversation} did not reach the agent: {listed}"));
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    /// The assignee's update of a task to `state` with these steps.
    pub fn update(
        &self,
        task: &Value,
        state: &str,
        steps: &Value,
        summary: Option<&str>,
    ) -> Result<u64, String> {
        let version = task["version"].as_u64().ok_or("task without version")? + 1;
        let mut body = json!({
            "conversation_id": task["conversation_id"], "task_id": task["task_id"],
            "version": version, "state": state, "steps": steps,
        });
        if let Some(summary) = summary {
            body["result_summary"] = json!(summary);
        }
        self.call("task", "task.update", &body)?;
        Ok(version)
    }

    /// The agent's MCP tool (`deploy/staging`, high risk) and one call of
    /// it: the approval it requires (single use, bound to its arguments).
    pub fn ask_approval(&self, args: &Value) -> Result<Value, String> {
        let server = json!({
            "server_id": "deploy", "command": "stdio-shell", "tool_name": "staging",
            "capability": "external_tool_invoke", "tool_scope": "staging", "risk_level": "high",
        });
        // A second task reuses the registered server.
        let _ = self.call("mcp", "mcp.server.add", &server);
        let started = self.tool(args)?;
        if started["status"] != "approval_required" {
            return Err(format!("the tool ran without an approval: {started}"));
        }
        Ok(started["approval"].clone())
    }

    /// Calls the agent's tool once.
    pub fn tool(&self, args: &Value) -> Result<Value, String> {
        self.call(
            "mcp",
            "mcp.tool.started",
            &json!({ "server_id": "deploy", "tool_name": "staging", "arguments": args,
                     "operation_origin": "ai_mcp" }),
        )
    }
}

/// Who an account is.
pub struct Who<'a> {
    pub account: &'a str,
    pub principal: &'a str,
    pub label: &'a str,
    pub root: [u8; 32],
    pub agent: bool,
}

/// 32 random bytes.
pub fn random() -> [u8; 32] {
    use std::io::Read as _;
    let mut bytes = [0_u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .expect("system randomness");
    bytes
}

/// The first `n` bytes as hex.
pub fn hex(bytes: &[u8], n: usize) -> String {
    bytes.iter().take(n).map(|b| format!("{b:02x}")).collect()
}

/// The grant id a task step names for an approval.
pub fn requested_grant(approval: &Value) -> String {
    approval["details"]["requested_grant_id"].as_str().map_or_else(
        || format!("grant_{}", approval["approval_id"].as_str().unwrap_or_default()),
        str::to_owned,
    )
}
