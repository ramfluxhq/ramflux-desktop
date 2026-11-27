// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Username lookups and claims as domain events: what a lookup found (or
//! why it could not be answered), and where a claim stands. Codes are the
//! directory's typed codes; the desktop shows them through its own text.

use ramflux_desktop_core::{AccountId, DomainEvent, UsernameClaim, UsernameLookup, methods};
use serde_json::Value;

use crate::bus::BusError;

/// A claim of `account` has started: it is being published.
pub fn claim_started(account: &str, name: &str) -> Vec<DomainEvent> {
    vec![DomainEvent::UsernameClaimUpdated {
        account: AccountId::new(account),
        claim: Some(UsernameClaim::Publishing { name: clean(name) }),
    }]
}

/// A lookup made for `account` has started: the previous answer no longer
/// applies.
pub fn lookup_started(account: Option<&AccountId>) -> Vec<DomainEvent> {
    vec![DomainEvent::UsernameLookupUpdated { account: account.cloned(), lookup: None }]
}

/// Clears the lookup and the claim of `account` (the account is locked, or
/// its scope ends).
pub fn account_cleared(account: &AccountId) -> Vec<DomainEvent> {
    vec![
        DomainEvent::UsernameLookupUpdated { account: Some(account.clone()), lookup: None },
        DomainEvent::UsernameClaimUpdated { account: account.clone(), claim: None },
    ]
}

/// The events of a finished `username.resolve` or `username.claim`, made for
/// `account` (`None` when no account was open).
pub fn outcome(
    account: Option<&AccountId>,
    method: &str,
    body: &Value,
    result: &Result<Value, BusError>,
) -> Vec<DomainEvent> {
    let name = clean(body["name"].as_str().unwrap_or_default());
    let lookup = |lookup| DomainEvent::UsernameLookupUpdated {
        account: account.cloned(),
        lookup: Some(lookup),
    };
    match (method, result) {
        (methods::USERNAME_RESOLVE, Ok(answer)) => vec![lookup(found(name, answer))],
        (methods::USERNAME_RESOLVE, Err(error)) => {
            let code = failure_code(error);
            let found = if code == "not_found" {
                UsernameLookup::NotFound { name }
            } else {
                UsernameLookup::Refused { name, code, retry_after_ms: retry_after(error) }
            };
            vec![lookup(found)]
        }
        (methods::USERNAME_CLAIM, _) => {
            let Some(account) = account else { return Vec::new() };
            let claim = match result {
                Ok(answer) => claimed(name, answer),
                Err(error) => Some(UsernameClaim::Failed {
                    name,
                    code: failure_code(error),
                    retry_after_ms: retry_after(error),
                }),
            };
            vec![DomainEvent::UsernameClaimUpdated { account: account.clone(), claim }]
        }
        _ => Vec::new(),
    }
}

/// A status read that reports a claim operation committed or aborted settles
/// the claim that waits on it (the claim state matches the operation id).
pub fn settled(account: &AccountId, status: &Value) -> Vec<DomainEvent> {
    let committed = match status.get("state").and_then(Value::as_str) {
        Some("committed") => true,
        Some("aborted") => false,
        _ => return Vec::new(),
    };
    let Some(operation_id) = status.get("operation_id").and_then(Value::as_str) else {
        return Vec::new();
    };
    vec![DomainEvent::UsernameClaimSettled {
        account: account.clone(),
        operation_id: operation_id.to_owned(),
        committed,
    }]
}

/// The name without a leading `@` and surrounding space.
fn clean(name: &str) -> String {
    name.trim().trim_start_matches('@').trim().to_owned()
}

/// What a found answer names: the name asked for, with the display name and
/// bio the account published, when it published them.
fn found(name: String, answer: &Value) -> UsernameLookup {
    let profile = &answer["profile"]["body"];
    let text = |key: &str| {
        profile[key].as_str().map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned)
    };
    UsernameLookup::Found { name, display_name: text("display_name"), bio: text("bio") }
}

/// Where a claim stands after the daemon's last answer. An answer without a
/// state is the no-op a prepare gives: nothing is claimed in progress. A
/// claim still pending when the wait ran out keeps its operation id, so a
/// later status read can settle it.
fn claimed(name: String, answer: &Value) -> Option<UsernameClaim> {
    match answer.get("state").and_then(Value::as_str) {
        Some("committed") => Some(UsernameClaim::Committed { name }),
        Some("aborted") => {
            Some(UsernameClaim::Failed { name, code: "aborted".to_owned(), retry_after_ms: None })
        }
        Some("pending") => Some(UsernameClaim::StillPublishing {
            name,
            operation_id: answer.get("operation_id").and_then(Value::as_str).map(str::to_owned),
        }),
        _ => None,
    }
}

/// The typed code of a refusal: the directory's own code when the daemon
/// gave one, else the daemon's code; transport and timeouts by name.
fn failure_code(error: &BusError) -> String {
    match error {
        BusError::Daemon { code, reason_code, .. } => {
            reason_code.clone().filter(|r| !r.is_empty()).unwrap_or_else(|| code.clone())
        }
        BusError::Transport(_) => "transport".to_owned(),
        BusError::Timeout => "timeout".to_owned(),
    }
}

fn retry_after(error: &BusError) -> Option<u64> {
    match error {
        BusError::Daemon { retry_after_ms, .. } => *retry_after_ms,
        _ => None,
    }
}
