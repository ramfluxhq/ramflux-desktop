// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! MCP approvals, grants and standing approvals.
//!
//! A desktop grant id is the daemon's approval id: a pending approval is a
//! pending grant, and the daemon's grant record that an approval produces is
//! the same desktop grant, approved (its daemon grant id is kept for
//! revocation).

use ramflux_desktop_core::{
    DomainEvent, Grant, GrantId, GrantState, Risk, StandingApproval, StandingApprovalId,
};
use serde_json::Value;

use super::{Translator, list, seconds, text};

fn risk(value: &Value) -> Risk {
    match value.as_str() {
        Some("critical") => Risk::Critical,
        Some("high") => Risk::High,
        Some("medium") => Risk::Medium,
        _ => Risk::Low,
    }
}

fn scope(record: &Value) -> String {
    let tool = format!("{}/{}", text(record, "server_id"), text(record, "tool_name"));
    match record["tool_scope"].as_str() {
        Some(extra) if !extra.is_empty() => format!("{tool} ({extra})"),
        _ => tool,
    }
}

impl Translator {
    /// A pending approval (listed or pushed) as a requested grant.
    pub(super) fn approval(&mut self, record: &Value) -> Option<DomainEvent> {
        let id = GrantId::new(text(record, "approval_id"));
        if id.as_str().is_empty() || self.grants.contains_key(&id) {
            return None;
        }
        if record["status"].as_str().is_some_and(|s| s != "pending") {
            return None;
        }
        self.grants.insert(id.clone(), GrantState::Pending);
        let args_hash = record["details"]["arguments_hash"].as_str().unwrap_or_default().to_owned();
        let expiry = seconds(&record["expires_at"]);
        self.bind(&id, &args_hash, expiry);
        Some(DomainEvent::GrantRequested(Grant {
            id,
            scope: scope(record),
            risk: risk(&record["risk_level"]),
            single_use: record["details"]["single_use"].as_bool().unwrap_or(false),
            args_hash,
            expiry,
            state: GrantState::Pending,
            failure: None,
        }))
    }

    pub(super) fn approval_list(&mut self, value: &Value) -> Vec<DomainEvent> {
        list(value, "approvals").iter().filter_map(|record| self.approval(record)).collect()
    }

    /// A decision on a known pending grant.
    pub(super) fn approval_decided(&mut self, body: &Value, state: GrantState) -> Vec<DomainEvent> {
        let id = GrantId::new(text(body, "approval_id"));
        if let Some(grant) = body["grant_id"].as_str() {
            self.daemon_grants.insert(id.to_string(), grant.to_owned());
        }
        self.resolve(&id, state)
    }

    pub(super) fn resolve(&mut self, id: &GrantId, state: GrantState) -> Vec<DomainEvent> {
        match self.grants.get(id) {
            Some(GrantState::Pending) => {
                self.grants.insert(id.clone(), state);
                vec![DomainEvent::GrantResolved { grant_id: id.clone(), state }]
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn revoked(&mut self, id: &GrantId) -> Vec<DomainEvent> {
        match self.grants.get(id) {
            Some(GrantState::Approved) => {
                self.grants.insert(id.clone(), GrantState::Revoked);
                vec![DomainEvent::GrantRevoked { grant_id: id.clone() }]
            }
            _ => Vec::new(),
        }
    }

    /// Approved grants. One the desktop has not seen pending is reported as
    /// requested then approved, which is what happened.
    pub(super) fn grant_list(&mut self, value: &Value) -> Vec<DomainEvent> {
        let mut events = Vec::new();
        for record in list(value, "grants") {
            let body = &record["signing_body"];
            let approval = text(body, "approval_id");
            let daemon_grant = text(record, "grant_id");
            if approval.is_empty() {
                continue;
            }
            self.daemon_grants.insert(approval.to_owned(), daemon_grant.to_owned());
            let id = GrantId::new(approval);
            if !self.grants.contains_key(&id) {
                self.grants.insert(id.clone(), GrantState::Pending);
                events.push(DomainEvent::GrantRequested(Grant {
                    id: id.clone(),
                    scope: scope(body),
                    risk: Risk::default(),
                    single_use: body["single_use"].as_bool().unwrap_or(false),
                    args_hash: body["arguments_hash"].as_str().unwrap_or_default().to_owned(),
                    expiry: seconds(&body["expires_at"]),
                    state: GrantState::Pending,
                    failure: None,
                }));
            }
            events.extend(self.resolve(&id, GrantState::Approved));
            if record["state"]["revoked"].as_bool().unwrap_or(false) {
                events.extend(self.revoked(&id));
            }
        }
        events
    }

    pub(super) fn standing_list(&mut self, value: &Value) -> Vec<DomainEvent> {
        let mut events = Vec::new();
        for record in list(value, "standing_approvals") {
            let approval = StandingApproval {
                id: StandingApprovalId::new(text(record, "standing_approval_id")),
                scope: scope(record),
                max_risk: risk(&record["risk_level"]),
                expiry: seconds(&record["expires_at"]),
                // The daemon reports no use count.
                uses: 0,
                revoked: record["revoked"].as_bool().unwrap_or(false),
            };
            if self.standing.get(&approval.id) != Some(&approval) {
                self.standing.insert(approval.id.clone(), approval.clone());
                events.push(DomainEvent::StandingApprovalUpserted(approval));
            }
        }
        events
    }
}
