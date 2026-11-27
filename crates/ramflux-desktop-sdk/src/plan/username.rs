// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Username effects: the lookup of a typed `@name` (`username.resolve`, one
//! read), and the claim or change of this account's name, which the bridge
//! runs as a job ([`ClaimJob`]): prepare on the daemon, authorize in this
//! process with the recovery phrase, submit the signed records, then poll
//! the status until the directory commits or aborts them.
//!
//! The recovery phrase is taken from the intent's secret and handed to the
//! job; it never enters a request body, a log line or a stored event.

use ramflux_desktop_core::SecretInput;
use serde_json::{Value, json};

use super::{Plan, call, refuse_invalid, text};
use crate::bus::BusRequest;

/// One claim or change, run by the bridge in order. The phrase is consumed
/// by the authorization step and dropped with it.
#[derive(Debug)]
pub struct ClaimJob {
    /// The account whose name changes.
    pub account: String,
    /// The name, without `@`.
    pub name: String,
    /// The recovery phrase that authorizes the change.
    pub phrase: SecretInput,
}

/// `username.resolve` `{name}`: the directory's answer for the name.
pub(super) fn resolve(acct: Option<&str>, body: &Value) -> Plan {
    let name = text(body, "name");
    if name.is_empty() {
        return refuse_invalid("no username given");
    }
    call(BusRequest::new(acct, "username", "username.resolve", json!({ "name": name })))
}

/// `username.claim` `{name}` with the recovery phrase as its secret.
pub(super) fn claim(acct: Option<&str>, body: &Value, mut secrets: Vec<SecretInput>) -> Plan {
    let Some(account) = acct else {
        return refuse_invalid("this action needs an unlocked account");
    };
    let name = text(body, "name");
    if name.is_empty() {
        return refuse_invalid("no username given");
    }
    let Some(phrase) = secrets.pop() else {
        return refuse_invalid("missing recovery phrase");
    };
    Plan::Username(Box::new(ClaimJob {
        account: account.to_owned(),
        name: name.to_owned(),
        phrase,
    }))
}
