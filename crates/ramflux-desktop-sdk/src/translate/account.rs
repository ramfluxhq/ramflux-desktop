// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Account status, devices and the integrations snapshot.

use std::collections::BTreeMap;

use ramflux_desktop_core::{
    AccountId, Device, DeviceId, DomainEvent, Integrations, LockState, McpServer, NodeId,
    ParticipantId, SecureSessions,
};
use serde_json::Value;

use super::{Scope, Translator, list, text};
use crate::directory::field;

/// The account stream's event announcing that the account's secure
/// sessions changed (`ready` once a restored device activated again). Its
/// body: `local_account_id`, `secure_sessions`.
pub const SECURE_SESSIONS_TOPIC: &str = "account.secure_sessions.changed";

/// Reads a `secure_sessions` field: `reestablishing`, or ready; `None`
/// when the body has no such field (nothing is said about them).
fn secure_sessions(value: &Value) -> Option<SecureSessions> {
    value.get("secure_sessions").map(|_| {
        if text(value, "secure_sessions") == "reestablishing" {
            SecureSessions::Reestablishing
        } else {
            SecureSessions::Ready
        }
    })
}

impl Translator {
    /// Whether this scope's own account is waiting for its networking.
    pub fn own_networking_pending(&self) -> bool {
        self.scope.account().and_then(|id| self.accounts.get(id)).is_some_and(|account| {
            account.networking == ramflux_desktop_core::Networking::Pending
                && account.lock_state == LockState::Unlocked
        })
    }

    pub(super) fn account_status(&mut self, id: &AccountId, value: &Value) -> Vec<DomainEvent> {
        let mut account = self.account_record(id);
        account.lock_state = LockState::Unlocked;
        account.node_id = value["node_id"].as_str().map(NodeId::new);
        if let Some(networking) = super::daemon::networking(value) {
            account.networking = networking;
        }
        if let Some(sessions) = secure_sessions(value) {
            account.secure_sessions = sessions;
        }
        if self.scope == Scope::Account(id.clone()) {
            let principal = field(value, "principal_id");
            if principal.is_some() {
                self.directory.self_principal.clone_from(&principal);
            }
            if let Some(commitment) = field(value, "principal_commitment") {
                self.directory.self_commitment = Some(commitment);
            }
            // The daemon names this account's own device in its status; it
            // wins over the local device that `device.list` reports.
            if let Some(device) = field(value, "device_id") {
                self.self_device = Some(device);
                self.status_device = true;
            }
            account.device_id.clone_from(&self.self_device);
            // The participant that is this account is its principal: contacts,
            // members and conversation participants are principals too.
            account.self_participant_id = principal.as_deref().map(ParticipantId::new);
            self.directory.relay_endpoint =
                account.node_id.as_ref().and_then(|node| self.node_relays.get(node)).cloned();
        }
        self.put_account(account)
    }

    /// Applies a body naming an account (`local_account_id`) and its
    /// `secure_sessions` (`account.secure_sessions.changed`, or
    /// `account.restored`, which carries it too).
    pub fn secure_sessions_changed(&mut self, body: &Value) -> Vec<DomainEvent> {
        let id = text(body, "local_account_id");
        if id.is_empty() {
            return Vec::new();
        }
        let Some(sessions) = secure_sessions(body) else { return Vec::new() };
        let mut account = self.account_record(&AccountId::new(id));
        account.secure_sessions = sessions;
        self.put_account(account)
    }

    /// The label this account's own identity row declares.
    pub(super) fn own_label(&mut self, label: &str) -> Vec<DomainEvent> {
        let Some(id) = self.scope.account().cloned() else { return Vec::new() };
        let mut account = self.account_record(&id);
        label.clone_into(&mut account.label);
        self.put_account(account)
    }

    pub(super) fn device_list(&mut self, value: &Value) -> Vec<DomainEvent> {
        let Some(account_id) = self.scope.account().cloned() else { return Vec::new() };
        let local = value["local_device_id"].as_str().map(str::to_owned);
        let mut events = Vec::new();
        if local.is_some() && local != self.self_device && !self.status_device {
            self.self_device.clone_from(&local);
            let mut account = self.account_record(&account_id);
            account.device_id.clone_from(&local);
            events.extend(self.put_account(account));
        }
        let listed: BTreeMap<DeviceId, Device> = list(value, "devices")
            .iter()
            .map(|d| {
                let id = DeviceId::new(text(d, "device_id"));
                let device = Device {
                    id: id.clone(),
                    account_id: account_id.clone(),
                    // The daemon keeps no device label; the id stands in.
                    label: id.to_string(),
                    this_device: d["is_local"].as_bool().unwrap_or(false),
                    revoked: false,
                    last_seen: None,
                };
                (id, device)
            })
            .collect();
        // A device that left the list was revoked; the record is kept.
        let revoked: Vec<Device> = self
            .devices
            .values()
            .filter(|d| !d.revoked && !listed.contains_key(&d.id))
            .map(|d| Device { revoked: true, ..d.clone() })
            .collect();
        for device in revoked.into_iter().chain(listed.into_values()) {
            if self.devices.get(&device.id) != Some(&device) {
                self.devices.insert(device.id.clone(), device.clone());
                events.push(DomainEvent::DeviceUpserted(device));
            }
        }
        events
    }

    /// `mcp.tool.list`: the tools of each server.
    pub(super) fn mcp_tools(&mut self, value: &Value) -> Vec<DomainEvent> {
        let mut servers: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for tool in list(value, "tools") {
            servers
                .entry(text(tool, "server_id").to_owned())
                .or_default()
                .push(text(tool, "tool_name").to_owned());
        }
        self.mcp_tools = Some(servers);
        self.integrations_changed()
    }

    /// `mcp.server.list` `server_details`: the registered servers and what
    /// was observed about their reachability (`reachable: null` while the
    /// daemon has not checked; a registered tool is not proof of it).
    pub(super) fn mcp_servers(&mut self, rows: &[Value]) -> Vec<DomainEvent> {
        self.mcp_servers = Some(rows.to_vec());
        self.integrations_changed()
    }

    fn integrations_changed(&mut self) -> Vec<DomainEvent> {
        let tools = self.mcp_tools.clone().unwrap_or_default();
        let mut servers: BTreeMap<String, McpServer> = BTreeMap::new();
        for row in self.mcp_servers.iter().flatten() {
            let Some(id) = field(row, "id") else { continue };
            servers.insert(
                id.clone(),
                McpServer { name: id, tools: Vec::new(), healthy: row["reachable"].as_bool() },
            );
        }
        for (server, names) in tools {
            servers
                .entry(server.clone())
                .or_insert_with(|| McpServer { name: server, tools: Vec::new(), healthy: None })
                .tools = names;
        }
        let integrations =
            Integrations { mcp_servers: servers.into_values().collect(), a2i_pending: Vec::new() };
        if self.integrations.as_ref() == Some(&integrations) {
            return Vec::new();
        }
        self.integrations = Some(integrations.clone());
        vec![DomainEvent::IntegrationsUpdated(integrations)]
    }
}

#[cfg(test)]
mod tests {
    use ramflux_desktop_core::Timestamp;
    use serde_json::json;

    use super::*;
    use crate::plan::fetch::Fetch;

    #[test]
    fn secure_sessions_come_with_the_status_and_change_with_the_announcement() {
        let id = AccountId::new("acct-main");
        let mut t = Translator::new(Scope::Account(id.clone()), "rfd.sock");
        let status = json!({ "local_account_id": "acct-main", "node_id": "n-1",
                             "secure_sessions": "reestablishing" });
        let _ = t.fetched(&Fetch::AccountStatus(id.clone()), &status, Timestamp(1));
        assert_eq!(t.accounts[&id].secure_sessions, SecureSessions::Reestablishing);
        // The announcement (also carried by `account.restored`).
        let ready = json!({ "local_account_id": "acct-main", "secure_sessions": "ready" });
        let events = t.secure_sessions_changed(&ready);
        assert_eq!(events.len(), 1);
        assert_eq!(t.accounts[&id].secure_sessions, SecureSessions::Ready);
        assert!(t.secure_sessions_changed(&ready).is_empty(), "unchanged: nothing said");
        assert!(t.secure_sessions_changed(&json!({})).is_empty(), "no account named");
        // A status without the field says nothing about them.
        let _ = t.secure_sessions_changed(&json!({ "local_account_id": "acct-main",
                                                   "secure_sessions": "reestablishing" }));
        let plain = json!({ "local_account_id": "acct-main", "node_id": "n-1" });
        let _ = t.fetched(&Fetch::AccountStatus(id.clone()), &plain, Timestamp(2));
        assert_eq!(t.accounts[&id].secure_sessions, SecureSessions::Reestablishing);
    }

    #[test]
    fn the_status_device_wins_over_the_listed_local_device() {
        let id = AccountId::new("acct-main");
        let mut t = Translator::new(Scope::Account(id.clone()), "rfd.sock");
        let listed = json!({ "local_device_id": "device-listed", "devices": [] });
        let _ = t.fetched(&Fetch::DeviceList, &listed, Timestamp(0));
        assert_eq!(t.self_device.as_deref(), Some("device-listed"));
        let status = json!({ "principal_id": "principal-me", "device_id": "device-status" });
        let _ = t.fetched(&Fetch::AccountStatus(id.clone()), &status, Timestamp(1));
        assert_eq!(t.self_device.as_deref(), Some("device-status"));
        let _ = t.fetched(&Fetch::DeviceList, &listed, Timestamp(2));
        assert_eq!(t.self_device.as_deref(), Some("device-status"), "the status stays the source");
        assert_eq!(t.accounts.get(&id).and_then(|a| a.device_id.as_deref()), Some("device-status"));
    }
}
