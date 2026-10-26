// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Full backup and restore effects (BACKUP-01, product
//! `docs/account_backup.md`; see `docs/contract.md`, "Full backup and
//! restore"):
//!
//! - `account.backup.create {output_path, allow_missing_attachments}` with
//!   the secret `passphrase`, in the scope of the account backed up;
//!   `allow_missing_attachments` is `false` unless the user explicitly
//!   allowed recording unavailable files;
//! - `account.restore.verify {input_path}` with the secret `passphrase`;
//! - `account.restore.install {job_id}`;
//! - `account.backup.cancel {job_id}`.
//!
//! Verify, install and cancel are daemon-level (no account): a restore may
//! be of an account that is not on this device. Each answers at once with
//! `{job: row}`; the job then runs in the daemon and reports each change as
//! `account.backup.job.changed` on the daemon-level job stream. The daemon
//! takes these requests only from an attended frontend's connection, the
//! one that follows that stream: the bridge sends them on it
//! ([`crate::bus::jobs`]).

use ramflux_desktop_core::{SecretInput, methods};
use serde_json::{Value, json};

use super::{Plan, call, refuse_invalid, text};
use crate::bus::BusRequest;

/// The `sdk_api` label of backup requests.
pub const API: &str = "account";

/// Plans a backup effect in the account scope `acct`.
pub fn backup_effect(
    acct: Option<&str>,
    method: &str,
    body: &Value,
    mut secrets: Vec<SecretInput>,
) -> Plan {
    let path = text(body, "path");
    let job = text(body, "job_id");
    match method {
        methods::BACKUP_CREATE | methods::RESTORE_VERIFY => {
            let Some(secret) = secrets.pop() else {
                return refuse_invalid("missing backup passphrase");
            };
            if path.is_empty() {
                return refuse_invalid("no backup file named");
            }
            let request = if method == methods::BACKUP_CREATE {
                // The intent names the account backed up; it must be the
                // scope's (the one whose connection is attended).
                let Some(account) = acct else {
                    return refuse_invalid("this action needs an unlocked account");
                };
                if text(body, "account_id") != account {
                    return refuse_invalid("the backup is of another account");
                }
                let allow = body["allow_missing_attachments"].as_bool().unwrap_or(false);
                BusRequest::new(
                    Some(account),
                    API,
                    "account.backup.create",
                    json!({ "output_path": path, "allow_missing_attachments": allow }),
                )
            } else {
                BusRequest::new(None, API, "account.restore.verify", json!({ "input_path": path }))
            };
            call(request.with_secret("passphrase", secret))
        }
        methods::RESTORE_INSTALL | methods::BACKUP_CANCEL if job.is_empty() => {
            refuse_invalid("no backup job named")
        }
        methods::RESTORE_INSTALL => {
            call(BusRequest::new(None, API, "account.restore.install", json!({ "job_id": job })))
        }
        methods::BACKUP_CANCEL => {
            call(BusRequest::new(None, API, "account.backup.cancel", json!({ "job_id": job })))
        }
        _ => refuse_invalid("no daemon counterpart"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::Plan;

    #[test]
    fn a_backup_is_created_for_the_account_the_intent_names_in_its_own_scope() {
        let body = json!({ "account_id": "acct-main", "path": "/b/x" });
        let secret = || vec![SecretInput::new("river stone lantern harbor")];
        let Plan::Call { request, .. } =
            backup_effect(Some("acct-main"), methods::BACKUP_CREATE, &body, secret())
        else {
            panic!("a call");
        };
        assert_eq!(request.account.as_deref(), Some("acct-main"));
        assert_eq!(request.body["output_path"], "/b/x");
        assert_eq!(request.body["allow_missing_attachments"], false);
        // Another account's backup is not sent on this scope's connection.
        assert!(matches!(
            backup_effect(Some("acct-other"), methods::BACKUP_CREATE, &body, secret()),
            Plan::Refuse { .. }
        ));
        assert!(matches!(
            backup_effect(None, methods::BACKUP_CREATE, &body, secret()),
            Plan::Refuse { .. }
        ));
        // A restore needs no account.
        let verify = json!({ "path": "/b/x" });
        let Plan::Call { request, .. } =
            backup_effect(None, methods::RESTORE_VERIFY, &verify, secret())
        else {
            panic!("a call");
        };
        assert_eq!(request.account, None);
        assert_eq!(request.body["input_path"], "/b/x");
    }
}
