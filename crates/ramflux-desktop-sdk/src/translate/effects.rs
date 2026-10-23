// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Answers to effects, and the audit record of confirmed ones.

use ramflux_desktop_core::{
    AccountId, AccountKind, Actor, AuditEntry, ContextId, Disappearing, DomainEvent, GrantId,
    GrantState, LockState, NodeId, ObjectId, ParticipantId, Timestamp, methods,
};
use serde_json::Value;

use super::daemon::node_from_profile;
use super::{Translator, text};

impl Translator {
    /// Translates the daemon's answer to a performed effect.
    pub fn effect_done(
        &mut self,
        method: &str,
        body: &Value,
        answer: &Value,
        now: Timestamp,
    ) -> Vec<DomainEvent> {
        let mut events = self.effect_events(method, body, answer, now);
        if !body["confirmed_by"].is_null() {
            events.push(audit(method, body, now));
        }
        events
    }

    fn effect_events(
        &mut self,
        method: &str,
        body: &Value,
        answer: &Value,
        now: Timestamp,
    ) -> Vec<DomainEvent> {
        let node_id = || NodeId::new(text(body, "node_id"));
        let context = || ContextId::new(text(body, "context_id"));
        match method {
            methods::NODE_ADD => {
                let default = self.default_node().map(str::to_owned);
                let mut node = node_from_profile(&answer["node"], default.as_deref(), false);
                if let Some(fingerprint) = answer["key_fingerprint"].as_str() {
                    node.fingerprint = Some(fingerprint.to_owned());
                }
                vec![self.upsert_node(node)]
            }
            methods::NODE_CONFIRM => {
                let default = self.default_node().map(str::to_owned);
                vec![self.upsert_node(node_from_profile(answer, default.as_deref(), true))]
            }
            methods::NODE_PROBE => {
                vec![DomainEvent::NodeProbed {
                    node_id: node_id(),
                    probe: Self::probe_result(answer, now),
                }]
            }
            methods::NODE_REMOVE => {
                self.nodes.remove(&node_id());
                vec![DomainEvent::NodeRemoved { node_id: node_id() }]
            }
            methods::NODE_SET_DEFAULT => {
                for node in self.nodes.values_mut() {
                    node.is_default = node.id == node_id();
                }
                vec![DomainEvent::NodeSettingsChanged {
                    node_id: node_id(),
                    enabled: None,
                    is_default: Some(true),
                }]
            }
            methods::NODE_SET_ENABLED => {
                let enabled = body["enabled"].as_bool();
                if let (Some(node), Some(enabled)) = (self.nodes.get_mut(&node_id()), enabled) {
                    node.enabled = enabled;
                }
                vec![DomainEvent::NodeSettingsChanged {
                    node_id: node_id(),
                    enabled,
                    is_default: None,
                }]
            }
            methods::ACCOUNT_UNLOCK | methods::ACCOUNT_BACKUP_IMPORT => {
                let id = AccountId::new(text(answer, "local_account_id"));
                self.opened_account(&id, answer, None)
            }
            methods::ACCOUNT_CREATE => {
                let id = AccountId::new(text(answer, "local_account_id"));
                let kind = serde_json::from_value(body["kind"].clone()).unwrap_or_default();
                let mut events = self.opened_account(&id, answer, Some(kind));
                let mut account = self.account_record(&id);
                account.node_id = Some(node_id());
                // The daemon keeps the label in the account manifest.
                let label = text(body, "label").trim();
                if !label.is_empty() {
                    label.clone_into(&mut account.label);
                }
                events.extend(self.put_account(account));
                events
            }
            methods::ACCOUNT_LOCK => {
                let id = AccountId::new(text(body, "account_id"));
                self.set_lock(&id, LockState::Locked)
            }
            methods::GRANT_APPROVE => {
                let id = GrantId::new(text(body, "grant_id"));
                if let Some(grant) = answer["grant_id"].as_str() {
                    self.daemon_grants.insert(id.to_string(), grant.to_owned());
                }
                let mut events = self.resolve(&id, GrantState::Approved);
                events.extend(self.approval_answer(&id, text(body, "args_hash"), answer, now));
                events
            }
            methods::GRANT_DENY => {
                self.resolve(&GrantId::new(text(body, "grant_id")), GrantState::Denied)
            }
            methods::GRANT_REVOKE => self.revoked(&GrantId::new(text(body, "grant_id"))),
            methods::CONTEXT_MUTE => {
                self.context_setting(&context(), body["muted"].as_bool(), None)
            }
            methods::CONTEXT_DISAPPEARING => {
                let timer = body["seconds"]
                    .as_u64()
                    .and_then(|s| u32::try_from(s).ok())
                    .map_or(Disappearing::Off, Disappearing::After);
                self.context_setting(&context(), None, Some(timer))
            }
            methods::CONTACT_ADD
            | methods::CONTACT_ACCEPT
            | methods::CONTACT_REJECT
            | methods::CONTACT_BLOCK
            | methods::CONTACT_UNBLOCK
            | methods::CONTACT_REMOVE => {
                // The daemon answers with the link record (the federated
                // accept wraps it in `link`).
                self.contact_record(answer.get("link").unwrap_or(answer))
            }
            methods::SEND_MESSAGE => {
                if let Some(id) = answer["message_id"].as_str() {
                    self.remember_sent(id, text(body, "text"));
                }
                Vec::new()
            }
            methods::OBJECT_SAVE => match answer["saved_to"].as_str() {
                Some(path) => self.object_saved(&ObjectId::new(text(body, "object_id")), path),
                None => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// Applies a desktop-local effect (pin) and returns its events.
    pub fn local(&mut self, method: &str, body: &Value) -> Vec<DomainEvent> {
        let context = ContextId::new(text(body, "context_id"));
        self.local_pin(&context, method == methods::CONTEXT_PIN)
    }

    fn opened_account(
        &mut self,
        id: &AccountId,
        answer: &Value,
        kind: Option<AccountKind>,
    ) -> Vec<DomainEvent> {
        let mut account = self.account_record(id);
        account.lock_state = LockState::Unlocked;
        if let Some(kind) = kind {
            account.kind = kind;
        }
        if let Some(device) = answer["device_id"].as_str() {
            account.device_id = Some(device.to_owned());
        }
        if let Some(principal) = answer["principal_id"].as_str() {
            account.self_participant_id = Some(ParticipantId::new(principal));
        }
        self.put_account(account)
    }
}

/// The audit record of an effect the attended human confirmed with a hold.
fn audit(method: &str, body: &Value, at: Timestamp) -> DomainEvent {
    let subject = [
        "grant_id",
        "approval_id",
        "node_id",
        "device_id",
        "moment_id",
        "context_id",
        "participant_id",
    ]
    .into_iter()
    .find_map(|field| body[field].as_str())
    .unwrap_or_default()
    .to_owned();
    let proposed_by =
        serde_json::from_value::<Actor>(body["proposed_by"].clone()).unwrap_or(Actor::Human);
    DomainEvent::AuditAppended(AuditEntry {
        at,
        action: method.to_owned(),
        subject,
        proposed_by,
        human_confirmed: true,
    })
}
