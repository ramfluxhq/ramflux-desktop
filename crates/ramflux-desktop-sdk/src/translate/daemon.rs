// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Daemon status, node profiles and the account list.

use ramflux_desktop_core::{
    Account, AccountId, DaemonInfo, DomainEvent, LockState, Node, NodeHealth, NodeId, PinSource,
    SourceStatus, Timestamp,
};
use serde_json::Value;

use super::{Translated, Translator, list, text};
use crate::plan::fetch::Fetch;

/// A daemon node profile as a desktop node.
///
/// `confirmed` is false for a discovery answer (`node.add`) and true for a
/// listed (persisted, pinned) profile.
pub(crate) fn node_from_profile(profile: &Value, default: Option<&str>, confirmed: bool) -> Node {
    let id = text(profile, "node_id");
    let endpoints = [
        &profile["gateway"]["quic_addr"],
        &profile["gateway"]["tcp_addr"],
        &profile["relay"]["quic_addr"],
        &profile["discovery_url"],
    ]
    .into_iter()
    .filter_map(Value::as_str)
    .filter(|e| !e.is_empty())
    .map(str::to_owned)
    .collect();
    let key = text(profile, "node_public_key");
    let fingerprint = (!key.is_empty()).then(|| {
        ramflux_protocol::hash_hex(ramflux_protocol::CLIENT_DISCOVERY_DOMAIN, key.as_bytes())
    });
    // The daemon has four pin sources; the desktop three. An operator pin
    // is configured out of band like a shipped one.
    let pin_source = match text(profile, "pin_source") {
        "invitation" => PinSource::Invite,
        "tofu" => PinSource::Manual,
        _ => PinSource::Bootstrap,
    };
    Node {
        id: NodeId::new(id),
        endpoints,
        fingerprint,
        pin_source,
        health: NodeHealth::Unknown,
        capabilities: list(profile, "client_capabilities")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        expires_at: super::seconds(&profile["expires_at"]),
        confirmed,
        builtin: profile["builtin"].as_bool().unwrap_or(false),
        enabled: profile["enabled"].as_bool().unwrap_or(true),
        is_default: default == Some(id),
        probe: None,
    }
}

impl Translator {
    pub(super) fn daemon_status(&mut self, value: &Value) -> Vec<DomainEvent> {
        let socket =
            value["socket_path"].as_str().map_or_else(|| self.socket.clone(), str::to_owned);
        let open = value["accounts"].as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
        vec![
            DomainEvent::SourceStatusChanged(SourceStatus::Connected),
            DomainEvent::DaemonInfoUpdated(DaemonInfo { socket_path: socket, open_accounts: open }),
        ]
    }

    pub(super) fn node_list(&mut self, value: &Value) -> Vec<DomainEvent> {
        let default = value["default_node_id"].as_str();
        let listed: Vec<Node> =
            list(value, "nodes").iter().map(|p| node_from_profile(p, default, true)).collect();
        let mut events = Vec::new();
        let gone: Vec<NodeId> = self
            .nodes
            .values()
            .filter(|n| n.confirmed && !listed.iter().any(|l| l.id == n.id))
            .map(|n| n.id.clone())
            .collect();
        for id in gone {
            self.nodes.remove(&id);
            events.push(DomainEvent::NodeRemoved { node_id: id });
        }
        for mut node in listed {
            if let Some(known) = self.nodes.get(&node.id) {
                // Keep what the list does not carry: the last probe.
                node.health = known.health;
                node.probe.clone_from(&known.probe);
                if known == &node {
                    continue;
                }
            }
            self.nodes.insert(node.id.clone(), node.clone());
            events.push(DomainEvent::NodeUpserted(node));
        }
        events
    }

    /// Records a node reported by an effect answer and returns its upsert.
    pub(super) fn upsert_node(&mut self, node: Node) -> DomainEvent {
        self.nodes.insert(node.id.clone(), node.clone());
        DomainEvent::NodeUpserted(node)
    }

    pub(super) fn default_node(&self) -> Option<&str> {
        self.nodes.values().find(|n| n.is_default).map(|n| n.id.as_str())
    }

    pub(super) fn account_list(&mut self, value: &Value) -> Translated {
        let open: Vec<AccountId> =
            list(value, "accounts").iter().filter_map(Value::as_str).map(AccountId::new).collect();
        let mut out = Translated::default();
        let locked_elsewhere: Vec<AccountId> = self
            .accounts
            .values()
            .filter(|a| a.lock_state == LockState::Unlocked && !open.contains(&a.id))
            .map(|a| a.id.clone())
            .collect();
        for id in locked_elsewhere {
            out.events.extend(self.set_lock(&id, LockState::Locked));
        }
        for id in open {
            let unknown = !self.accounts.contains_key(&id);
            out.events.extend(self.set_lock(&id, LockState::Unlocked));
            if unknown {
                out.follow.push(Fetch::AccountStatus(id));
            }
        }
        out
    }

    /// The account record as known so far.
    pub(super) fn account_record(&self, id: &AccountId) -> Account {
        self.accounts.get(id).cloned().unwrap_or_else(|| Account {
            id: id.clone(),
            // The daemon keeps no account label: the user's own label when
            // this session created it, else the local account id.
            label: self.labels.get(id).cloned().unwrap_or_else(|| id.to_string()),
            kind: ramflux_desktop_core::AccountKind::default(),
            node_id: None,
            device_id: None,
            lock_state: LockState::Locked,
            self_participant_id: None,
        })
    }

    /// Emits the account when it differs from what was reported.
    pub(super) fn put_account(&mut self, account: Account) -> Vec<DomainEvent> {
        if self.accounts.get(&account.id) == Some(&account) {
            return Vec::new();
        }
        self.accounts.insert(account.id.clone(), account.clone());
        vec![DomainEvent::AccountUpdated(account)]
    }

    pub(super) fn set_lock(&mut self, id: &AccountId, lock_state: LockState) -> Vec<DomainEvent> {
        let mut account = self.account_record(id);
        account.lock_state = lock_state;
        self.put_account(account)
    }

    pub(super) fn remember_label(&mut self, id: &AccountId, label: &str) {
        if !label.trim().is_empty() {
            self.labels.insert(id.clone(), label.trim().to_owned());
        }
    }

    pub(super) fn probe_result(value: &Value, at: Timestamp) -> ramflux_desktop_core::NodeProbe {
        let gateway_ok = value["gateway"]["app"].as_str() == Some("ok");
        let rtt = value["gateway"]["rtt_ms"].as_u64().and_then(|ms| u32::try_from(ms).ok());
        let relay_status = text(&value["relay"], "status");
        ramflux_desktop_core::NodeProbe {
            at,
            gateway: ramflux_desktop_core::ProbeOutcome {
                ok: gateway_ok,
                latency_ms: if gateway_ok { rtt } else { None },
                detail: text(&value["discovery"], "status").to_owned(),
            },
            relay: ramflux_desktop_core::ProbeOutcome {
                ok: relay_status == "ok",
                latency_ms: None,
                detail: relay_status.to_owned(),
            },
        }
    }
}
