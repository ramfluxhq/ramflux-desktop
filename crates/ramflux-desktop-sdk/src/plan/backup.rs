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
                if acct.is_none() {
                    return refuse_invalid("this action needs an unlocked account");
                }
                let allow = body["allow_missing_attachments"].as_bool().unwrap_or(false);
                BusRequest::new(
                    acct,
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
