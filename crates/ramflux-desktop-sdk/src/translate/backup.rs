// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Full backup and restore jobs (BACKUP-01, product
//! `docs/account_backup.md`; see `docs/contract.md`, "Full backup and
//! restore").
//!
//! A job row comes as the answer to a job request (`{job: row}`), as the
//! body of each `account.backup.job.changed` event (on the daemon-level job
//! stream, [`crate::bus::jobs`]), and in the `jobs` of an
//! `account.backup.jobs` read (with every snapshot, and with every poll
//! while a job runs). Rows are reported once per `seq`: a replayed or older
//! row changes nothing. A row missing what a job needs, or naming an
//! unknown kind, state or progress category, is dropped (nothing is
//! guessed).
//!
//! Row fields: `job_id`, `kind` (`backup` / `restore`), `state`
//! (`preparing`, `writing`, `verifying`, `verified`, `installing`, `done`,
//! `failed`, `cancelled`), `seq`, `path`, `created_at`, `finished_at` (Unix
//! seconds), `progress {category, items_done, items_total, bytes_done,
//! bytes_total}` (always present; a backup `preparing` on `attachments`
//! with an item total is fetching missing files), `counts {contacts,
//! conversations, messages, groups, surfaces, tasks, attachments,
//! attachment_bytes}` (a backup's snapshot, a restore's authenticated
//! archive), `archive_bytes`, `archive {local_account_id, account_label,
//! created_at, merge}` (a verified restore), `restored` and `kept_newer`
//! (per-category counts, a finished restore), `attachments {verified,
//! mismatched, missing, unavailable_at_source, local_missing_kept}` (a
//! verified, failed or finished restore), `error {code, needed_bytes,
//! available_bytes, unavailable_count, unavailable_attachments,
//! unavailable_next_cursor}` (a failed job).
//!
//! A restore that finished calls for the account's reads again: what it
//! installed is shown as it is shown after a reload.
//!
//! Jobs live in the daemon's memory: a daemon that restarted lists none of
//! the jobs the one before it ran. The job stream says when that happened
//! (it has to start over from the head), and the listing read then settles
//! every job reported as running: one no longer listed is reported
//! `Failed`, `Interrupted`, so the next job may start.

use std::collections::BTreeSet;

use ramflux_desktop_core::{
    AccountId, AttachmentCheck, BackupCategory, BackupCounts, BackupFailure, BackupJob,
    BackupJobId, BackupJobKind, BackupPhase, BackupProgress, BackupSummary, DomainEvent, Listing,
    RestorePreview, RestoreReport,
};
use serde_json::Value;

use super::{Scope, Translated, Translator, list, seconds, text};
use crate::plan::fetch::Fetch;

/// The job topic (daemon-level job stream only): its body is the job row,
/// read directly.
pub const TOPIC: &str = crate::bus::jobs::TOPIC;

/// The account stream's last event after a restore installed into the
/// account (after the usual change events): the account is read again.
pub const RESTORED_TOPIC: &str = "account.restored";

/// The typed error codes of a job and the reasons a job request is refused
/// with (`reason_code`), each with its locale key.
pub const ERROR_KEYS: [(&str, &str); 17] = [
    ("wrong_passphrase", "backend.backup.wrong_passphrase"),
    ("tampered", "backend.backup.tampered"),
    ("truncated", "backend.backup.truncated"),
    ("unsupported_version", "backend.backup.unsupported_version"),
    ("insufficient_space", "backend.backup.insufficient_space"),
    ("unreadable", "backend.backup.unreadable"),
    ("not_writable", "backend.backup.not_writable"),
    ("staging_expired", "backend.backup.staging_expired"),
    ("interrupted", "backend.backup.interrupted"),
    ("attachment_mismatch", "backend.backup.attachment_mismatch"),
    ("attachments_unavailable", "backend.backup.attachments_unavailable"),
    ("identity_rollback", "backend.backup.identity_rollback"),
    ("root_conflict", "backend.backup.root_conflict"),
    ("busy", "backend.backup.busy"),
    ("account_locked", "backend.backup.account_locked"),
    ("unknown_job", "backend.backup.unknown_job"),
    ("invalid_job_phase", "backend.backup.invalid_job_phase"),
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

fn progress(value: &Value) -> Option<BackupProgress> {
    Some(BackupProgress {
        category: category(&value["category"])?,
        items_done: number(value, "items_done"),
        items_total: value["items_total"].as_u64(),
        bytes_done: number(value, "bytes_done"),
        bytes_total: value["bytes_total"].as_u64(),
    })
}

/// A file check (`attachments`), when the row has one.
fn checks(value: &Value) -> Option<AttachmentCheck> {
    value.is_object().then(|| AttachmentCheck {
        verified: number(value, "verified"),
        mismatched: number(value, "mismatched"),
        missing: number(value, "missing"),
        unavailable_at_source: number(value, "unavailable_at_source"),
        local_missing_kept: number(value, "local_missing_kept"),
    })
}

/// A failed job's typed error; `checks` is the row's file check.
pub fn failure(error: &Value, checks: Option<AttachmentCheck>) -> BackupFailure {
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
        "attachment_mismatch" => BackupFailure::AttachmentMismatch { checks },
        "attachments_unavailable" => BackupFailure::AttachmentsUnavailable {
            count: error["unavailable_count"].as_u64().unwrap_or_else(|| {
                u64::try_from(list(error, "unavailable_attachments").len()).unwrap_or(u64::MAX)
            }),
            selectors: list(error, "unavailable_attachments")
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            next_cursor: error["unavailable_next_cursor"].as_str().map(str::to_owned),
        },
        "identity_rollback" => BackupFailure::IdentityRollback,
        "root_conflict" => BackupFailure::RootConflict,
        other => BackupFailure::Other(other.to_owned()),
    }
}

/// The locale key of a refused job request's typed reason, if it is a job
/// error code or a refusal reason (a refusal carries only its
/// `reason_code`; figures such as the space short come on a failed job's
/// row, never on a refusal).
pub fn refusal_message(reason_code: &str) -> Option<String> {
    ERROR_KEYS.iter().find(|(code, _)| *code == reason_code).map(|(_, key)| (*key).to_owned())
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
    let file_checks = checks(&row["attachments"]);
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
                attachments: file_checks.unwrap_or_default(),
            })
        })
        .flatten();
    let report = (kind == BackupJobKind::Restore && done).then(|| RestoreReport {
        restored: counts(&row["restored"]),
        kept_newer: counts(&row["kept_newer"]),
        attachments: file_checks.unwrap_or_default(),
    });
    Some(BackupJob {
        id: BackupJobId::new(id),
        kind,
        path: path.to_owned(),
        phase,
        version: row["seq"].as_u64()?,
        started_at: row.get("created_at").and_then(seconds)?,
        finished_at: row.get("finished_at").and_then(seconds),
        progress: progress(&row["progress"])?,
        summary,
        preview,
        report,
        failure: (phase == BackupPhase::Failed).then(|| failure(&row["error"], file_checks)),
    })
}

impl Translator {
    /// Translates one job row. A restore that just finished calls for the
    /// account's reads (what it installed).
    pub fn backup_job_row(&mut self, row: &Value) -> Translated {
        let Some(job) = job_of(row) else { return Translated::default() };
        if self.reported_jobs.get(&job.id).is_some_and(|last| last.version >= job.version) {
            return Translated::default();
        }
        let installed = job.kind == BackupJobKind::Restore
            && job.phase == BackupPhase::Done
            && self.reported_jobs.get(&job.id).is_some_and(|last| last.phase != job.phase);
        if job.phase.is_terminal() {
            self.lost_jobs.remove(&job.id);
        }
        self.reported_jobs.insert(job.id.clone(), job.clone());
        let follow = match (&self.scope, installed) {
            (Scope::Account(id), true) => Fetch::account_snapshot(id),
            // With no account open, the restored account joins the list.
            (Scope::Daemon, true) => vec![Fetch::AccountList],
            _ => Vec::new(),
        };
        Translated { events: vec![DomainEvent::BackupJobUpdated(Box::new(job))], follow }
    }

    /// Translates an `account.backup.jobs` answer (`jobs`, newest first):
    /// reported oldest first, so the store sees them in the order they
    /// started. After the job stream started over ([`Self::backup_stream_resynced`]),
    /// a job reported as running that the daemon no longer lists ended with
    /// the daemon that ran it: it is reported `Failed`, `Interrupted`.
    pub fn backup_job_list(&mut self, value: &Value) -> Translated {
        let mut out = Translated::default();
        let rows = list(value, "jobs");
        for row in rows.iter().rev() {
            let translated = self.backup_job_row(row);
            out.events.extend(translated.events);
            for fetch in translated.follow {
                if !out.follow.contains(&fetch) {
                    out.follow.push(fetch);
                }
            }
        }
        let listed: BTreeSet<BackupJobId> =
            rows.iter().map(|row| BackupJobId::new(text(row, "job_id"))).collect();
        for id in std::mem::take(&mut self.lost_jobs) {
            let Some(last) = self.reported_jobs.get(&id) else { continue };
            if listed.contains(&id) || last.phase.is_terminal() {
                continue;
            }
            let mut lost = last.clone();
            lost.phase = BackupPhase::Failed;
            lost.version += 1;
            lost.failure = Some(BackupFailure::Interrupted);
            self.reported_jobs.insert(id, lost.clone());
            out.events.push(DomainEvent::BackupJobUpdated(Box::new(lost)));
        }
        out
    }

    /// The daemon refused the job listing for good (not a transient failure):
    /// the jobs are declared listed all the same, so the UI does not wait
    /// for a listing that will not come. Empty once declared.
    pub fn backup_jobs_given_up(&mut self) -> Vec<DomainEvent> {
        self.listed(&[Listing::BackupJobs]).into_iter().collect()
    }

    /// The job stream had to start over from the daemon's head: the cursor
    /// it followed is beyond what the daemon holds, so the daemon restarted
    /// (its jobs are in memory only, and go with it). The jobs reported as
    /// running are settled by the next listing ([`Self::backup_job_list`]).
    pub fn backup_stream_resynced(&mut self) {
        self.lost_jobs = self
            .reported_jobs
            .iter()
            .filter(|(_, job)| !job.phase.is_terminal())
            .map(|(id, _)| id.clone())
            .collect();
    }

    /// Whether a job this source reported has not ended (polls read the
    /// jobs while one runs).
    pub fn backup_running(&self) -> bool {
        self.reported_jobs.values().any(|job| !job.phase.is_terminal())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A row as the product writes it: `fields` go through the product's own
    /// job type (`ramflux_sdk::backup::BackupJob`), so a field it does not
    /// have, or of another shape, fails here.
    fn product_row(fields: Value) -> Value {
        let job: ramflux_sdk::backup::BackupJob =
            serde_json::from_value(fields).expect("a product job row");
        serde_json::to_value(job).expect("row json")
    }

    fn counts(messages: u64, contacts: u64) -> Value {
        json!({
            "contacts": contacts, "conversations": 3, "messages": messages, "groups": 2,
            "surfaces": 1, "tasks": 1, "attachments": 4, "attachment_bytes": 4_096,
        })
    }

    fn base(kind: &str, state: &str) -> Value {
        json!({
            "job_id": "j-1", "kind": kind, "state": state, "seq": 5, "path": "/b/x",
            "created_at": 1_790_000_000, "finished_at": null,
            "progress": { "category": "messages", "items_done": 0, "items_total": null,
                          "bytes_done": 10, "bytes_total": 40 },
        })
    }

    #[test]
    fn a_merged_restore_reports_per_category_and_the_full_file_check() {
        let mut row = base("restore", "done");
        row["finished_at"] = json!(1_790_000_100);
        row["counts"] = counts(10, 5);
        row["archive_bytes"] = json!(9_000);
        row["archive"] = json!({ "local_account_id": "acct-main", "account_label": "Me",
                                 "created_at": 1_789_000_000, "merge": true });
        row["restored"] = counts(8, 4);
        row["kept_newer"] = json!({ "contacts": 1, "conversations": 0, "messages": 2,
            "groups": 0, "surfaces": 0, "tasks": 0, "attachments": 0, "attachment_bytes": 0 });
        row["attachments"] = json!({ "verified": 4, "mismatched": 0, "missing": 2,
            "unavailable_at_source": 2, "local_missing_kept": 3 });
        let job = job_of(&product_row(row)).expect("a job");
        assert_eq!(
            (job.kind, job.phase, job.version),
            (BackupJobKind::Restore, BackupPhase::Done, 5)
        );
        let preview = job.preview.expect("the archive");
        assert!(preview.merge);
        assert_eq!(preview.file_bytes, 9_000);
        assert_eq!(preview.attachments.unavailable_at_source, 2);
        let report = job.report.expect("the report");
        assert_eq!(report.kept_newer.messages, 2);
        assert_eq!(report.kept_newer.contacts, 1);
        assert_eq!(report.restored.messages, 8);
        assert_eq!(report.attachments.local_missing_kept, 3);
        assert!(report.attachments.all_verified(), "only recorded omissions are missing");
        assert_eq!(report.attachments.unapproved_missing(), 0);
    }

    #[test]
    fn a_backup_fetching_files_then_failing_on_unavailable_ones_is_typed() {
        let mut fetching = base("backup", "preparing");
        fetching["progress"] = json!({ "category": "attachments", "items_done": 3,
            "items_total": 9, "bytes_done": 0, "bytes_total": null });
        let job = job_of(&product_row(fetching)).expect("a job");
        assert!(job.progress.is_fetching(job.phase));
        assert_eq!(job.progress.fraction(), Some(1.0 / 3.0));

        let mut failed = base("backup", "failed");
        failed["finished_at"] = json!(1_790_000_100);
        failed["error"] = json!({ "code": "attachments_unavailable", "unavailable_count": 70,
            "unavailable_attachments": ["obj-1", "obj-2"], "unavailable_next_cursor": "obj-2" });
        let job = job_of(&product_row(failed)).expect("a job");
        assert_eq!(
            job.failure,
            Some(BackupFailure::AttachmentsUnavailable {
                count: 70,
                selectors: vec!["obj-1".to_owned(), "obj-2".to_owned()],
                next_cursor: Some("obj-2".to_owned()),
            })
        );
    }

    #[test]
    fn every_product_failure_code_is_typed_and_has_a_locale_key() {
        let typed = [
            ("wrong_passphrase", BackupFailure::WrongPassphrase),
            ("tampered", BackupFailure::Tampered),
            ("truncated", BackupFailure::Truncated),
            ("unsupported_version", BackupFailure::UnsupportedVersion),
            ("unreadable", BackupFailure::Unreadable),
            ("not_writable", BackupFailure::NotWritable),
            ("staging_expired", BackupFailure::StagingExpired),
            ("interrupted", BackupFailure::Interrupted),
            ("identity_rollback", BackupFailure::IdentityRollback),
            ("root_conflict", BackupFailure::RootConflict),
        ];
        for (code, expected) in typed {
            let mut row = base("restore", "failed");
            row["error"] = json!({ "code": code });
            assert_eq!(job_of(&product_row(row)).unwrap().failure, Some(expected), "{code}");
            assert!(refusal_message(code).is_some(), "{code}");
        }
        let mut mismatch = base("restore", "failed");
        mismatch["error"] = json!({ "code": "attachment_mismatch" });
        mismatch["attachments"] = json!({ "verified": 3, "mismatched": 1, "missing": 0 });
        let checks = AttachmentCheck { verified: 3, mismatched: 1, ..AttachmentCheck::default() };
        assert_eq!(
            job_of(&product_row(mismatch)).unwrap().failure,
            Some(BackupFailure::AttachmentMismatch { checks: Some(checks) })
        );
        let mut space = base("backup", "failed");
        space["error"] =
            json!({ "code": "insufficient_space", "needed_bytes": 9, "available_bytes": 4 });
        assert_eq!(
            job_of(&product_row(space)).unwrap().failure,
            Some(BackupFailure::InsufficientSpace { needed: Some(9), available: Some(4) })
        );
        for reason in ["busy", "account_locked", "unknown_job", "invalid_job_phase"] {
            assert_eq!(
                refusal_message(reason).as_deref(),
                Some(format!("backend.backup.{reason}").as_str())
            );
        }
    }

    #[test]
    fn a_running_job_the_restarted_daemon_no_longer_lists_is_interrupted() {
        let mut t = Translator::new(Scope::Account(AccountId::new("acct-main")), "sock");
        let running = product_row(base("restore", "verifying"));
        assert_eq!(t.backup_job_row(&running).events.len(), 1);
        assert!(t.backup_running());
        // A listing without it, with no stream resync, changes nothing (it
        // may be older than the job).
        assert!(t.backup_job_list(&json!({ "jobs": [] })).events.is_empty());
        // The same daemon, after a resync, still lists it: nothing changes.
        t.backup_stream_resynced();
        assert!(t.backup_job_list(&json!({ "jobs": [running] })).events.is_empty());
        assert!(t.backup_running());
        // A restarted daemon lists nothing of it: it ended with the daemon.
        t.backup_stream_resynced();
        let out = t.backup_job_list(&json!({ "jobs": [] }));
        let [DomainEvent::BackupJobUpdated(lost)] = out.events.as_slice() else {
            panic!("expected the lost job, got {:?}", out.events);
        };
        assert_eq!(lost.id, BackupJobId::new("j-1"));
        assert_eq!((lost.phase, lost.version), (BackupPhase::Failed, 6));
        assert_eq!(lost.failure, Some(BackupFailure::Interrupted));
        assert!(!t.backup_running());
        // Settled once: a later listing says nothing more about it.
        assert!(t.backup_job_list(&json!({ "jobs": [] })).events.is_empty());
    }

    #[test]
    fn a_row_with_an_unknown_category_is_dropped() {
        let mut row = base("backup", "writing");
        row["progress"]["category"] = json!("everything");
        assert!(job_of(&row).is_none());
    }
}
