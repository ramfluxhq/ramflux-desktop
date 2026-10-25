// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Full backup and restore effects (BACKUP-01, assumed contract; see
//! `docs/contract.md`, "Full backup and restore"):
//!
//! - `account.backup.create {output_path}` with the secret `passphrase`;
//! - `account.restore.verify {input_path}` with the secret `passphrase`;
//! - `account.restore.install {job_id}`;
//! - `account.backup.cancel {job_id}`.
//!
//! Each answers at once with `{job: row}`; the job then runs in the daemon
//! and reports each change as `account.backup.job.changed`. All are sent in
//! the scope of the account the user is in: its stream carries the job's
//! changes, also for a restore of another account.

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
            let (daemon, request) = if method == methods::BACKUP_CREATE {
                ("account.backup.create", json!({ "output_path": path }))
            } else {
                ("account.restore.verify", json!({ "input_path": path }))
            };
            call(BusRequest::new(acct, API, daemon, request).with_secret("passphrase", secret))
        }
        methods::RESTORE_INSTALL | methods::BACKUP_CANCEL if job.is_empty() => {
            refuse_invalid("no backup job named")
        }
        methods::RESTORE_INSTALL => {
            call(BusRequest::new(acct, API, "account.restore.install", json!({ "job_id": job })))
        }
        methods::BACKUP_CANCEL => {
            call(BusRequest::new(acct, API, "account.backup.cancel", json!({ "job_id": job })))
        }
        _ => refuse_invalid("no daemon counterpart"),
    }
}
