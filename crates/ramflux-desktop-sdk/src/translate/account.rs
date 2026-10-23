// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Account status, devices, contacts and the integrations snapshot.

use std::collections::BTreeMap;

use ramflux_desktop_core::{
    AccountId, Device, DeviceId, DomainEvent, Integrations, LockState, McpServer, NodeId,
    Participant, ParticipantId, ParticipantKind, SafetyState,
};
use serde_json::Value;

use super::{Scope, Translator, list, text};

impl Translator {
    pub(super) fn account_status(&mut self, id: &AccountId, value: &Value) -> Vec<DomainEvent> {
        let mut account = self.account_record(id);
        account.lock_state = LockState::Unlocked;
        account.node_id = value["node_id"].as_str().map(NodeId::new);
        if self.scope == Scope::Account(id.clone()) {
            self.principal = value["principal_id"].as_str().map(str::to_owned);
            account.device_id = self.self_device.clone();
            account.self_participant_id = self.self_device.as_deref().map(ParticipantId::new);
        }
        self.put_account(account)
    }

    pub(super) fn device_list(&mut self, value: &Value) -> Vec<DomainEvent> {
        let Some(account_id) = self.scope.account().cloned() else { return Vec::new() };
        let local = value["local_device_id"].as_str().map(str::to_owned);
        let mut events = Vec::new();
        if local.is_some() && local != self.self_device {
            self.self_device.clone_from(&local);
            let mut account = self.account_record(&account_id);
            account.device_id.clone_from(&local);
            account.self_participant_id = local.as_deref().map(ParticipantId::new);
            events.extend(self.put_account(account));
            if let Some(me) = &local {
                let label = self.account_record(&account_id).label;
                events.extend(self.participant(me, &label, false));
            }
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

    pub(super) fn contact_list(&mut self, value: &Value) -> Vec<DomainEvent> {
        let me = self.principal.clone().unwrap_or_default();
        let mut events = Vec::new();
        for link in list(value, "contacts") {
            let requester = text(link, "requester_id");
            let other = if requester == me { text(link, "target_id") } else { requester };
            if other.is_empty() {
                continue;
            }
            let accepted =
                text(link, "state") == "accepted" && !link["blocked"].as_bool().unwrap_or(false);
            events.extend(self.participant(other, other, accepted));
        }
        events
    }

    /// Upserts a participant the daemon named. The daemon reports no display
    /// names or kinds; the id is shown and the kind stays the neutral `Human`.
    pub(super) fn participant(
        &mut self,
        id: &str,
        display: &str,
        contact: bool,
    ) -> Vec<DomainEvent> {
        let id = ParticipantId::new(id);
        match self.participants.get(&id) {
            Some(known) if *known == contact || !contact => return Vec::new(),
            _ => {}
        }
        self.participants.insert(id.clone(), contact);
        vec![DomainEvent::ParticipantUpserted(Participant {
            id,
            kind: ParticipantKind::Human,
            display: display.to_owned(),
            safety: SafetyState::Unverified,
            contact,
            safety_number: None,
        })]
    }

    pub(super) fn mcp_tools(&mut self, value: &Value) -> Vec<DomainEvent> {
        let mut servers: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for tool in list(value, "tools") {
            servers
                .entry(text(tool, "server_id").to_owned())
                .or_default()
                .push(text(tool, "tool_name").to_owned());
        }
        let integrations = Integrations {
            mcp_servers: servers
                .into_iter()
                // The daemon reports no server health.
                .map(|(name, tools)| McpServer { name, tools, healthy: false })
                .collect(),
            a2i_pending: Vec::new(),
        };
        if self.integrations.as_ref() == Some(&integrations) {
            return Vec::new();
        }
        self.integrations = Some(integrations.clone());
        vec![DomainEvent::IntegrationsUpdated(integrations)]
    }
}
