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
use crate::bus::{BusError, IoCause};

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
        BusError::Daemon { code, message, cause, .. } => classify(code, message, cause.as_ref()),
    }
}

/// The refusal of a failed export. A file-system refusal is told by the
/// operating-system cause the daemon reports (`cause {kind, raw_os_error}`),
/// never by its English message; a refusal that names none is `Other`. The
/// one refusal with no structured form, the peer-credential check (the
/// daemon sends it as a plain `ValidationFailed`), is told by that check's
/// own message.
fn classify(code: &str, message: &str, cause: Option<&IoCause>) -> AgentExportRefusal {
    match code {
        "NetworkingPending" => return AgentExportRefusal::NotConnected,
        "CustodySetupInvalid" => return AgentExportRefusal::NotCustodian,
        "CustodyVaultUnavailable" => return AgentExportRefusal::CredentialsGone,
        "AccountNotOpen" => return AgentExportRefusal::OwnerLocked,
        _ => {}
    }
    if let Some(cause) = cause {
        return match (cause.kind.as_str(), cause.raw_os_error) {
            ("AlreadyExists", _) | (_, Some(17)) => AgentExportRefusal::FileExists,
            ("NotFound", _) | (_, Some(2)) => AgentExportRefusal::LocationMissing,
            ("PermissionDenied" | "ReadOnlyFilesystem" | "IsADirectory" | "NotADirectory", _)
            | (_, Some(13 | 30 | 21 | 20)) => AgentExportRefusal::LocationDenied,
            _ => AgentExportRefusal::Other,
        };
    }
    if message.to_ascii_lowercase().contains("peer credential") {
        return AgentExportRefusal::NotOwnerSession;
    }
    AgentExportRefusal::Other
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

    fn io(kind: &str, errno: Option<i32>) -> BusError {
        BusError::Daemon {
            code: "ValidationFailed".to_owned(),
            message: "words that may change".to_owned(),
            reason_code: None,
            details: None,
            cause: Some(IoCause { kind: kind.to_owned(), raw_os_error: errno }),
        }
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
    fn a_file_system_refusal_is_told_by_its_cause_whatever_the_words() {
        let cases = [
            (io("AlreadyExists", Some(17)), AgentExportRefusal::FileExists),
            (io("AlreadyExists", None), AgentExportRefusal::FileExists),
            (io("Other", Some(17)), AgentExportRefusal::FileExists),
            (io("NotFound", Some(2)), AgentExportRefusal::LocationMissing),
            (io("PermissionDenied", Some(13)), AgentExportRefusal::LocationDenied),
            (io("IsADirectory", None), AgentExportRefusal::LocationDenied),
            (io("Other", Some(30)), AgentExportRefusal::LocationDenied),
            (io("TimedOut", Some(60)), AgentExportRefusal::Other),
        ];
        for (error, expected) in cases {
            assert_eq!(export_refusal(&error), expected, "{error:?}");
        }
    }

    #[test]
    fn without_a_cause_the_words_of_a_file_system_refusal_are_not_read() {
        // The same words the daemon uses today, but no structured cause:
        // not guessed from the text.
        for message in [
            "io error: File exists (os error 17)",
            "io error: No such file or directory (os error 2)",
            "io error: Permission denied (os error 13)",
        ] {
            assert_eq!(
                export_refusal(&daemon("ValidationFailed", message)),
                AgentExportRefusal::Other,
                "{message}"
            );
        }
    }

    #[test]
    fn every_refusal_has_its_own_locale_key() {
        let keys: BTreeSet<&str> = ALL.into_iter().map(export_refusal_key).collect();
        assert_eq!(keys.len(), ALL.len());
        assert!(keys.iter().all(|k| key::ALL.contains(k)), "each key is in the table");
    }
}
