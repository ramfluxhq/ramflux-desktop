// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! AI participants an owner custodies (DESK-AGENT-01): who custodies each,
//! and what became of exporting its credentials.
//!
//! The account list names an AI participant's kind. It names the custodian
//! only when a daemon reports one (`custodian_account_id`); the desktop
//! otherwise knows the links its own ceremonies made (remembered, see
//! [`crate::custodians`]) and says nothing about the rest.
//!
//! `account.export_agent_credentials` answers `{account, path, exported}`.
//! A refusal reaches the desktop as one of the product's error codes
//! (`docs/local_bus_errors.md`) or, for the file system, as the operating
//! system's words inside `ValidationFailed`; each is mapped to an
//! [`AgentExportRefusal`], and the message of the failed effect is that
//! refusal's locale key, never the daemon's English. The daemon's text is
//! read, never shown. Nothing here sees a credential: the bundle is written
//! by the daemon.

use std::collections::{BTreeMap, BTreeSet};

use ramflux_desktop_core::{
    AccountId, AgentExport, AgentExportPhase, AgentExportRefusal, DomainEvent, FailureCode, methods,
};
use serde_json::Value;

use super::reason::key;
use super::{Translator, text};
use crate::bus::BusError;

/// What the translator keeps about AI participants.
#[derive(Debug, Default)]
pub(super) struct Agents {
    /// AI participant to custodian, as known.
    custodians: BTreeMap<AccountId, AccountId>,
    /// The links already told to the store.
    announced: BTreeSet<(AccountId, AccountId)>,
}

/// The refusal a daemon error means for an export.
pub fn export_refusal(error: &BusError) -> AgentExportRefusal {
    match error {
        BusError::Timeout => AgentExportRefusal::Timeout,
        BusError::Transport(_) => AgentExportRefusal::Transport,
        BusError::Daemon { code, message, .. } => classify(code, message),
    }
}

/// The refusal a failed effect's code and message mean.
fn classify(code: &str, message: &str) -> AgentExportRefusal {
    let said = message.to_ascii_lowercase();
    match code {
        "NetworkingPending" => AgentExportRefusal::NotConnected,
        "CustodySetupInvalid" => AgentExportRefusal::NotCustodian,
        "CustodyVaultUnavailable" => AgentExportRefusal::CredentialsGone,
        "AccountNotOpen" => AgentExportRefusal::OwnerLocked,
        _ if said.contains("peer credential") => AgentExportRefusal::NotOwnerSession,
        _ if said.contains("file exists") || said.contains("os error 17") => {
            AgentExportRefusal::FileExists
        }
        _ if said.contains("no such file") || said.contains("os error 2)") => {
            AgentExportRefusal::LocationMissing
        }
        _ if said.contains("permission denied")
            || said.contains("read-only")
            || said.contains("is a directory")
            || said.contains("not a directory") =>
        {
            AgentExportRefusal::LocationDenied
        }
        _ => AgentExportRefusal::Other,
    }
}

/// The locale key that says `refusal`.
pub fn export_refusal_key(refusal: AgentExportRefusal) -> &'static str {
    match refusal {
        AgentExportRefusal::NoLocation => key::AGENT_EXPORT_NO_LOCATION,
        AgentExportRefusal::NotOwnerSession => key::AGENT_EXPORT_NOT_OWNER_SESSION,
        AgentExportRefusal::OwnerLocked => key::AGENT_EXPORT_OWNER_LOCKED,
        AgentExportRefusal::NotCustodian => key::AGENT_EXPORT_NOT_CUSTODIAN,
        AgentExportRefusal::NotConnected => key::AGENT_EXPORT_NOT_CONNECTED,
        AgentExportRefusal::FileExists => key::AGENT_EXPORT_FILE_EXISTS,
        AgentExportRefusal::LocationDenied => key::AGENT_EXPORT_LOCATION_DENIED,
        AgentExportRefusal::LocationMissing => key::AGENT_EXPORT_LOCATION_MISSING,
        AgentExportRefusal::CredentialsGone => key::AGENT_EXPORT_CREDENTIALS_GONE,
        AgentExportRefusal::Timeout => key::AGENT_EXPORT_TIMEOUT,
        AgentExportRefusal::Transport => key::AGENT_EXPORT_TRANSPORT,
        AgentExportRefusal::Other => key::AGENT_EXPORT_OTHER,
    }
}

impl Translator {
    /// Custodian links remembered from earlier sessions: told to the store
    /// once the accounts are listed.
    pub fn restore_custodians(&mut self, links: BTreeMap<AccountId, AccountId>) {
        self.agents.custodians.extend(links);
    }

    /// The link a ceremony made, as an event.
    pub(super) fn custodian_made(
        &mut self,
        agent: &AccountId,
        custodian: &AccountId,
    ) -> Vec<DomainEvent> {
        self.agents.custodians.insert(agent.clone(), custodian.clone());
        self.announce_custodian(agent)
    }

    /// The custodian of the listed AI participant `agent`: the one the
    /// daemon reported, else the one this desktop knows.
    pub(super) fn custodian_listed(&mut self, agent: &AccountId, row: &Value) -> Vec<DomainEvent> {
        let reported = ["custodian_account_id", "custodian_account", "custodian"]
            .into_iter()
            .find_map(|field| row[field].as_str().filter(|id| !id.is_empty()));
        if let Some(custodian) = reported {
            self.agents.custodians.insert(agent.clone(), AccountId::new(custodian));
        }
        self.announce_custodian(agent)
    }

    fn announce_custodian(&mut self, agent: &AccountId) -> Vec<DomainEvent> {
        let Some(custodian) = self.agents.custodians.get(agent).cloned() else {
            return Vec::new();
        };
        if !self.agents.announced.insert((agent.clone(), custodian.clone())) {
            return Vec::new();
        }
        vec![DomainEvent::CustodianKnown { agent: agent.clone(), custodian }]
    }

    /// The daemon wrote the bundle.
    pub(super) fn export_done(&mut self, body: &Value, answer: &Value) -> Vec<DomainEvent> {
        let path = answer["path"].as_str().unwrap_or_else(|| text(body, "path"));
        vec![DomainEvent::AgentExportUpdated(AgentExport {
            account_id: AccountId::new(text(body, "account")),
            path: path.to_owned(),
            phase: AgentExportPhase::Exported,
        })]
    }

    /// The export did not happen. `message` is the failed effect's message,
    /// the refusal's locale key (see [`Translator::agent_failure`]).
    pub(super) fn export_failed(
        &self,
        body: &Value,
        code: &FailureCode,
        message: &str,
    ) -> Vec<DomainEvent> {
        let refusal = match code {
            FailureCode::Timeout => AgentExportRefusal::Timeout,
            FailureCode::Transport => AgentExportRefusal::Transport,
            _ => ALL
                .into_iter()
                .find(|refusal| export_refusal_key(*refusal) == message)
                .unwrap_or(AgentExportRefusal::Other),
        };
        vec![DomainEvent::AgentExportUpdated(AgentExport {
            account_id: AccountId::new(text(body, "account")),
            path: text(body, "path").to_owned(),
            phase: AgentExportPhase::Refused(refusal),
        })]
    }

    /// The message of a failed AI participant request: the refusal's key.
    pub(super) fn agent_failure(&self, method: &str, error: &BusError) -> Option<String> {
        (method == methods::AGENT_EXPORT_CREDENTIALS)
            .then(|| export_refusal_key(export_refusal(error)).to_owned())
    }
}

/// Every refusal an export can come to.
const ALL: [AgentExportRefusal; 12] = [
    AgentExportRefusal::NoLocation,
    AgentExportRefusal::NotOwnerSession,
    AgentExportRefusal::OwnerLocked,
    AgentExportRefusal::NotCustodian,
    AgentExportRefusal::NotConnected,
    AgentExportRefusal::FileExists,
    AgentExportRefusal::LocationDenied,
    AgentExportRefusal::LocationMissing,
    AgentExportRefusal::CredentialsGone,
    AgentExportRefusal::Timeout,
    AgentExportRefusal::Transport,
    AgentExportRefusal::Other,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn daemon(code: &str, message: &str) -> BusError {
        BusError::daemon(code, message)
    }

    #[test]
    fn each_refusal_the_daemon_makes_is_told_apart() {
        let cases = [
            ("CustodySetupInvalid", "custody setup invalid", AgentExportRefusal::NotCustodian),
            ("NetworkingPending", "networking is pending", AgentExportRefusal::NotConnected),
            ("CustodyVaultUnavailable", "vault", AgentExportRefusal::CredentialsGone),
            ("AccountNotOpen", "account not open: a", AgentExportRefusal::OwnerLocked),
            (
                "ValidationFailed",
                "local bus peer credential check failed",
                AgentExportRefusal::NotOwnerSession,
            ),
            (
                "ValidationFailed",
                "io error: File exists (os error 17)",
                AgentExportRefusal::FileExists,
            ),
            (
                "ValidationFailed",
                "io error: No such file or directory (os error 2)",
                AgentExportRefusal::LocationMissing,
            ),
            (
                "ValidationFailed",
                "io error: Permission denied (os error 13)",
                AgentExportRefusal::LocationDenied,
            ),
            ("ValidationFailed", "something new", AgentExportRefusal::Other),
        ];
        for (code, message, expected) in cases {
            assert_eq!(export_refusal(&daemon(code, message)), expected, "{code}: {message}");
        }
        assert_eq!(export_refusal(&BusError::Timeout), AgentExportRefusal::Timeout);
        assert_eq!(
            export_refusal(&BusError::Transport("gone".to_owned())),
            AgentExportRefusal::Transport
        );
    }

    #[test]
    fn every_refusal_has_its_own_locale_key() {
        let keys: BTreeSet<&str> = ALL.into_iter().map(export_refusal_key).collect();
        assert_eq!(keys.len(), ALL.len());
        assert!(keys.iter().all(|k| key::ALL.contains(k)), "each key is in the table");
    }
}
