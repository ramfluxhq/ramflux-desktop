// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Full backup and restore jobs (BACKUP-01, assumed contract; see
//! `docs/contract.md`, "Full backup and restore").
//!
//! A job row comes as the answer to a job effect (`{job: row}`), as the
//! body of each `account.backup.job.changed` event (stream or push), and in
//! the `jobs` of an `account.backup.jobs` read (with every snapshot, and with
//! every poll while a job runs). Rows are reported once per `seq`: a
//! replayed or older row changes nothing. A row missing what a job needs,
//! or naming an unknown kind or state, is dropped (nothing is guessed).
//!
//! Row fields: `job_id`, `kind` (`backup` / `restore`), `state`
//! (`preparing`, `writing`, `verifying`, `verified`, `installing`, `done`,
//! `failed`, `cancelled`), `seq`, `path`, `created_at`, `finished_at` (Unix
//! seconds), `progress {category, items_done, items_total, bytes_done,
//! bytes_total}`, `counts {contacts, conversations, messages, groups,
//! surfaces, tasks, attachments, attachment_bytes}` (a finished backup's or
//! a verified archive's content), `archive_bytes`, `archive
//! {local_account_id, account_label, created_at, merge}` (a verified
//! restore), `restored {counts…}`, `kept_newer`, `attachments {verified,
//! mismatched, missing}` (a finished restore), `error {code, needed_bytes,
//! available_bytes}` (a failed one).
//!
//! A restore that finished calls for the account's reads again: what it
//! installed is shown as it is shown after a reload.

use ramflux_desktop_core::{
    AccountId, AttachmentCheck, BackupCategory, BackupCounts, BackupFailure, BackupJob,
    BackupJobId, BackupJobKind, BackupPhase, BackupProgress, BackupSummary, DomainEvent,
    RestorePreview, RestoreReport,
};
use serde_json::Value;

use super::reason::keyed;
use super::{Scope, Translated, Translator, list, seconds, text};
use crate::plan::fetch::Fetch;

/// The job topic: its body is the job row, read directly.
pub const TOPIC: &str = "account.backup.job.changed";

/// The typed error codes of a job (and of a refused job request), each
/// with its locale key.
pub const ERROR_KEYS: [(&str, &str); 9] = [
    ("wrong_passphrase", "backend.backup.wrong_passphrase"),
    ("tampered", "backend.backup.tampered"),
    ("truncated", "backend.backup.truncated"),
    ("unsupported_version", "backend.backup.unsupported_version"),
    ("insufficient_space", "backend.backup.insufficient_space"),
    ("unreadable", "backend.backup.unreadable"),
    ("not_writable", "backend.backup.not_writable"),
    ("staging_expired", "backend.backup.staging_expired"),
    ("interrupted", "backend.backup.interrupted"),
];

fn kind(row: &Value) -> Option<BackupJobKind> {
    match text(row, "kind") {
        "backup" => Some(BackupJobKind::Backup),
        "restore" => Some(BackupJobKind::Restore),
        _ => None,
    }
}

fn phase(row: &Value) -> Option<BackupPhase> {
    Some(match text(row, "state") {
        "preparing" => BackupPhase::Preparing,
        "writing" => BackupPhase::Writing,
        "verifying" => BackupPhase::Verifying,
        "verified" => BackupPhase::Verified,
        "installing" => BackupPhase::Installing,
        "done" => BackupPhase::Done,
        "failed" => BackupPhase::Failed,
        "cancelled" => BackupPhase::Cancelled,
        _ => return None,
    })
}

fn category(value: &Value) -> Option<BackupCategory> {
    Some(match value.as_str()? {
        "identity" => BackupCategory::Identity,
        "contacts" => BackupCategory::Contacts,
        "conversations" => BackupCategory::Conversations,
        "messages" => BackupCategory::Messages,
        "groups" => BackupCategory::Groups,
        "surfaces" => BackupCategory::Surfaces,
        "tasks" => BackupCategory::Tasks,
        "settings" => BackupCategory::Settings,
        "attachments" => BackupCategory::Attachments,
        _ => return None,
    })
}

fn number(value: &Value, field: &str) -> u64 {
    value[field].as_u64().unwrap_or(0)
}

fn counts(value: &Value) -> BackupCounts {
    BackupCounts {
        contacts: number(value, "contacts"),
        conversations: number(value, "conversations"),
        messages: number(value, "messages"),
        groups: number(value, "groups"),
        surfaces: number(value, "surfaces"),
        tasks: number(value, "tasks"),
        attachments: number(value, "attachments"),
        attachment_bytes: number(value, "attachment_bytes"),
    }
}

fn progress(value: &Value) -> BackupProgress {
    BackupProgress {
        category: category(&value["category"]),
        items_done: number(value, "items_done"),
        items_total: value["items_total"].as_u64(),
        bytes_done: number(value, "bytes_done"),
        bytes_total: value["bytes_total"].as_u64(),
    }
}

/// A job's typed error.
pub fn failure(error: &Value) -> BackupFailure {
    match text(error, "code") {
        "wrong_passphrase" => BackupFailure::WrongPassphrase,
        "tampered" => BackupFailure::Tampered,
        "truncated" => BackupFailure::Truncated,
        "unsupported_version" => BackupFailure::UnsupportedVersion,
        "insufficient_space" => BackupFailure::InsufficientSpace {
            needed: error["needed_bytes"].as_u64(),
            available: error["available_bytes"].as_u64(),
        },
        "unreadable" => BackupFailure::Unreadable,
        "not_writable" => BackupFailure::NotWritable,
        "staging_expired" => BackupFailure::StagingExpired,
        "interrupted" => BackupFailure::Interrupted,
        other => BackupFailure::Other(other.to_owned()),
    }
}

/// The locale key of a refused job request's typed reason (with the space
/// figures of `insufficient_space`), if the reason is a job error code.
pub fn refusal_message(reason_code: &str, details: Option<&Value>) -> Option<String> {
    let (_, key) = ERROR_KEYS.iter().find(|(code, _)| *code == reason_code)?;
    let details = details.cloned().unwrap_or(Value::Null);
    let figure = |field: &str| details[field].as_u64().map(|n| n.to_string()).unwrap_or_default();
    Some(if reason_code == "insufficient_space" {
        keyed(
            key,
            &[("needed", &figure("needed_bytes")), ("available", &figure("available_bytes"))],
        )
    } else {
        (*key).to_owned()
    })
}

/// The job a row describes.
pub fn job_of(row: &Value) -> Option<BackupJob> {
    let id = text(row, "job_id");
    let path = text(row, "path");
    if id.is_empty() || path.is_empty() {
        return None;
    }
    let kind = kind(row)?;
    let phase = phase(row)?;
    let done = phase == BackupPhase::Done;
    let summary = (kind == BackupJobKind::Backup && done).then(|| BackupSummary {
        counts: counts(&row["counts"]),
        file_bytes: number(row, "archive_bytes"),
    });
    let archive = &row["archive"];
    let preview = (kind == BackupJobKind::Restore && archive.is_object())
        .then(|| {
            Some(RestorePreview {
                account_id: AccountId::new(
                    Some(text(archive, "local_account_id")).filter(|id| !id.is_empty())?,
                ),
                account_label: archive["account_label"].as_str().map(str::to_owned),
                created_at: archive.get("created_at").and_then(seconds)?,
                counts: counts(&row["counts"]),
                file_bytes: number(row, "archive_bytes"),
                merge: archive["merge"].as_bool().unwrap_or(false),
            })
        })
        .flatten();
    let report = (kind == BackupJobKind::Restore && done).then(|| RestoreReport {
        restored: counts(&row["restored"]),
        kept_newer: number(row, "kept_newer"),
        attachments: AttachmentCheck {
            verified: number(&row["attachments"], "verified"),
            mismatched: number(&row["attachments"], "mismatched"),
            missing: number(&row["attachments"], "missing"),
        },
    });
    Some(BackupJob {
        id: BackupJobId::new(id),
        kind,
        path: path.to_owned(),
        phase,
        version: row["seq"].as_u64()?,
        started_at: row.get("created_at").and_then(seconds)?,
        finished_at: row.get("finished_at").and_then(seconds),
        progress: progress(&row["progress"]),
        summary,
        preview,
        report,
        failure: (phase == BackupPhase::Failed).then(|| failure(&row["error"])),
    })
}

impl Translator {
    /// Translates one job row. A restore that just finished calls for the
    /// account's reads (what it installed).
    pub fn backup_job_row(&mut self, row: &Value) -> Translated {
        let Some(job) = job_of(row) else { return Translated::default() };
        if self.reported_jobs.get(&job.id).is_some_and(|(seq, _)| *seq >= job.version) {
            return Translated::default();
        }
        let installed = job.kind == BackupJobKind::Restore
            && job.phase == BackupPhase::Done
            && self.reported_jobs.get(&job.id).is_some_and(|(_, phase)| *phase != job.phase);
        self.reported_jobs.insert(job.id.clone(), (job.version, job.phase));
        let follow = match (&self.scope, installed) {
            (Scope::Account(id), true) => Fetch::account_snapshot(id),
            _ => Vec::new(),
        };
        Translated { events: vec![DomainEvent::BackupJobUpdated(job)], follow }
    }

    /// Translates an `account.backup.jobs` answer (`jobs`).
    pub fn backup_job_list(&mut self, value: &Value) -> Translated {
        let mut out = Translated::default();
        for row in list(value, "jobs") {
            let translated = self.backup_job_row(row);
            out.events.extend(translated.events);
            for fetch in translated.follow {
                if !out.follow.contains(&fetch) {
                    out.follow.push(fetch);
                }
            }
        }
        out
    }

    /// Whether a job this source reported has not ended (polls read the
    /// jobs while one runs).
    pub fn backup_running(&self) -> bool {
        self.reported_jobs.values().any(|(_, phase)| !phase.is_terminal())
    }
}
