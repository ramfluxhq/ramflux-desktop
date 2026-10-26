// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The fake daemon's full backup and restore: the BACKUP-01 job API as the
//! product implements it (product `docs/account_backup.md`; desktop
//! `docs/contract.md`, "Full backup and restore").
//!
//! Requests:
//!
//! - `account.backup.create {output_path, passphrase,
//!   allow_missing_attachments}` (account scoped) → `{job}`, `preparing`;
//! - `account.restore.verify {input_path, passphrase}` → `{job}`,
//!   `verifying`;
//! - `account.restore.install {job_id}` → `{job}`, `installing` (only a
//!   `verified` restore);
//! - `account.backup.cancel {job_id}` → `{job}`, `cancelled` (until a
//!   restore installs);
//! - `account.backup.jobs {}` → `{jobs: [rows]}`, newest first; with
//!   `{job_id, missing_after}` → the next page of a failed backup's
//!   unavailable files `{job, unavailable_attachments, unavailable_count,
//!   unavailable_next_cursor}`.
//!
//! Jobs are the daemon's, not an account's: one runs at a time
//! (`reason_code: busy`), verify, install and cancel need no account, and
//! every change is published as `account.backup.job.changed` on the
//! daemon-level job stream through the publisher the test installs. The
//! fake bus takes the four job requests only from a connection that opened
//! that stream as an attended frontend ([`crate::fake_bus`]). A refusal is
//! `BackupRefused` with the reason as `reason_code`.
//!
//! A job moves one step each time the test calls [`FakeBackup::step`] (so a
//! test or a screenshot can hold it in any phase). A backup first fetches
//! the files the account references but does not hold yet
//! ([`FakeBackup::missing_files`]): those that stay unavailable fail it
//! with `attachments_unavailable`, unless it was started with
//! `allow_missing_attachments: true`, when the archive records them as
//! unavailable at the source. It then writes its categories (identity,
//! contacts, conversations, messages, groups, surfaces, tasks, settings,
//! attachments). A restore reads and verifies the categories, fails on any
//! file that does not match or is missing without having been recorded, and
//! waits verified for the user; installing it reports what was restored and,
//! merging into an account already here, what was kept because it was newer
//! here, per category.
//!
//! Archives are what the test declares ([`FakeBackup::archive`]): the
//! passphrase that opens one, the account and counts it holds, and a defect.
//! A backup the fake writes becomes such an archive. Disk space can be made
//! short ([`FakeBackup::set_free_space`]).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use serde_json::{Value, json};

use crate::fake_bus::Reply;

/// The topic of job changes.
pub const TOPIC: &str = "account.backup.job.changed";

/// The job requests the daemon takes only from an attended connection.
pub const ATTENDED_METHODS: [&str; 4] = [
    "account.backup.create",
    "account.restore.verify",
    "account.restore.install",
    "account.backup.cancel",
];

/// Delivers one job row on the daemon-level job stream.
pub type Publisher = Box<dyn Fn(Value) + Send + Sync>;

/// The categories, in the order a job works through them.
pub const CATEGORIES: [&str; 9] = [
    "identity",
    "contacts",
    "conversations",
    "messages",
    "groups",
    "surfaces",
    "tasks",
    "settings",
    "attachments",
];

/// The largest page of unavailable file selectors the daemon sends.
pub const MISSING_PAGE: usize = 64;

/// What is wrong with an archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Defect {
    /// A chunk was changed: authentication fails halfway.
    Tampered,
    /// The file ends early: the last chunk is missing.
    Truncated,
    /// Installing it as a new account would roll its identity back (a
    /// merge keeps the live identity and is not refused).
    IdentityRollback,
    /// Its identity root differs from the account's here.
    RootConflict,
}

/// What an archive holds.
#[derive(Debug, Clone)]
pub struct Archive {
    /// The passphrase that opens it.
    pub passphrase: String,
    /// The account it holds (its local id) and name.
    pub account_id: String,
    /// The account's name.
    pub label: String,
    /// When it was made (Unix seconds).
    pub created_at: i64,
    /// Contacts, conversations, messages, groups, surfaces, tasks,
    /// attachments (the files it holds) and the attachments' bytes.
    pub counts: Counts,
    /// What is wrong with it.
    pub defect: Option<Defect>,
    /// Files whose bytes do not match their hashes.
    pub mismatched: u64,
    /// Files missing that the backup did not record as unavailable.
    pub missing: u64,
    /// Files the backup recorded as unavailable at its source (allowed
    /// explicitly when it was made).
    pub unavailable_at_source: u64,
}

impl Archive {
    /// An intact archive of `account` holding `counts`, opened by
    /// `passphrase`.
    pub fn new(passphrase: &str, account: &str, label: &str, counts: Counts) -> Self {
        Self {
            passphrase: passphrase.to_owned(),
            account_id: account.to_owned(),
            label: label.to_owned(),
            created_at: 1_789_000_000,
            counts,
            defect: None,
            mismatched: 0,
            missing: 0,
            unavailable_at_source: 0,
        }
    }
}

/// Item counts of an archive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Contacts.
    pub contacts: u64,
    /// Conversations.
    pub conversations: u64,
    /// Messages.
    pub messages: u64,
    /// Groups.
    pub groups: u64,
    /// Surfaces.
    pub surfaces: u64,
    /// Tasks.
    pub tasks: u64,
    /// Attachments.
    pub attachments: u64,
    /// The attachments' bytes.
    pub attachment_bytes: u64,
}

impl Counts {
    fn json(self) -> Value {
        json!({
            "contacts": self.contacts, "conversations": self.conversations,
            "messages": self.messages, "groups": self.groups, "surfaces": self.surfaces,
            "tasks": self.tasks, "attachments": self.attachments,
            "attachment_bytes": self.attachment_bytes,
        })
    }

    /// What the archive's size would be (the fake's own measure).
    fn archive_bytes(self) -> u64 {
        let items = self.contacts
            + self.conversations
            + self.messages
            + self.groups
            + self.surfaces
            + self.tasks
            + self.attachments;
        self.attachment_bytes + items * 512 + 4096
    }
}

/// Files a backup of an account finds missing locally.
#[derive(Debug, Clone, Copy, Default)]
struct Missing {
    /// Fetched before the snapshot.
    fetchable: u64,
    /// Still unavailable after the fetch.
    unavailable: u64,
}

#[derive(Debug, Clone)]
struct Job {
    kind: &'static str,
    path: String,
    state: &'static str,
    seq: u64,
    /// The order it started in (the newest first in a listing).
    order: u64,
    created_at: i64,
    finished_at: Option<i64>,
    /// Backup: preparing steps taken (fetching).
    fetched: u64,
    /// Categories done.
    done: usize,
    /// Installing: categories installed.
    installed: usize,
    archive: Archive,
    /// A backup's missing files, and whether recording them was allowed.
    missing: Missing,
    allow_missing: bool,
    /// A restore of a file the fake does not hold.
    unreadable: bool,
    /// The passphrase does not open the archive.
    wrong_passphrase: bool,
    merge: bool,
    /// Merging: files this device had not downloaded yet.
    local_pending: u64,
    error: Option<Value>,
    checks: Option<Value>,
}

impl Job {
    fn ended(&self) -> bool {
        matches!(self.state, "done" | "failed" | "cancelled")
    }

    fn progress(&self) -> Value {
        let steps = CATEGORIES.len();
        let total_bytes = self.archive.counts.archive_bytes();
        if self.kind == "backup" && self.state == "preparing" {
            let total = self.missing.fetchable + self.missing.unavailable;
            return json!({
                "category": "attachments", "items_done": self.fetched.min(total),
                "items_total": total, "bytes_done": 0, "bytes_total": null,
            });
        }
        let done = if self.state == "installing" { self.installed } else { self.done };
        let category = CATEGORIES[done.min(steps - 1)];
        let bytes = total_bytes * u64::try_from(done.min(steps)).unwrap_or(0) / steps as u64;
        json!({
            "category": category, "items_done": 0, "items_total": null,
            "bytes_done": bytes, "bytes_total": total_bytes,
        })
    }

    fn row(&self, id: &str) -> Value {
        let counts = self.archive.counts;
        let mut row = json!({
            "job_id": id, "kind": self.kind, "state": self.state, "seq": self.seq,
            "path": self.path, "created_at": self.created_at, "finished_at": self.finished_at,
            "progress": self.progress(),
        });
        let steps = CATEGORIES.len();
        let backup_counts = self.kind == "backup" && matches!(self.state, "writing" | "done")
            || self.kind == "backup" && self.state == "failed" && self.done > 0;
        let verified = self.kind == "restore" && self.done == steps && self.error.is_none()
            || self.kind == "restore" && self.state == "cancelled" && self.done == steps;
        if backup_counts || verified {
            row["counts"] = counts.json();
        }
        if (self.kind == "backup" && self.state == "done") || verified {
            row["archive_bytes"] = json!(counts.archive_bytes());
        }
        if verified {
            row["archive"] = json!({
                "local_account_id": self.archive.account_id,
                "account_label": self.archive.label,
                "created_at": self.archive.created_at,
                "merge": self.merge,
            });
        }
        if let Some(checks) = &self.checks {
            row["attachments"] = checks.clone();
        }
        if self.kind == "restore" && self.state == "done" {
            // Merging: some items are newer here and kept, per category.
            let kept = if self.merge {
                Counts {
                    contacts: counts.contacts.min(1),
                    messages: counts.messages.min(2),
                    ..Counts::default()
                }
            } else {
                Counts::default()
            };
            let restored = Counts {
                contacts: counts.contacts - kept.contacts,
                messages: counts.messages - kept.messages,
                ..counts
            };
            row["restored"] = restored.json();
            row["kept_newer"] = kept.json();
        }
        if let Some(error) = &self.error {
            row["error"] = error.clone();
        }
        row
    }

    fn fail(&mut self, now: i64, error: Value) {
        self.state = "failed";
        self.finished_at = Some(now);
        self.error = Some(error);
    }

    /// The archive's file check after verification.
    fn file_checks(&self) -> Value {
        let a = &self.archive;
        json!({
            "verified": a.counts.attachments - a.mismatched,
            "mismatched": a.mismatched,
            "missing": a.missing + a.unavailable_at_source,
            "unavailable_at_source": a.unavailable_at_source,
            "local_missing_kept": 0,
        })
    }
}

#[derive(Default)]
struct Inner {
    jobs: BTreeMap<String, Job>,
    archives: BTreeMap<String, Archive>,
    /// Accounts on this device (a restore of one merges).
    accounts: BTreeSet<String>,
    /// What a backup of an account holds.
    holdings: BTreeMap<String, Counts>,
    /// Files a backup of an account finds missing.
    missing: BTreeMap<String, Missing>,
    /// Files an account here references but has not downloaded.
    local_pending: BTreeMap<String, u64>,
    /// Free disk space, when short.
    free_space: Option<u64>,
    next: u64,
    clock: i64,
}

/// Full backup and restore of the fake daemon.
pub struct FakeBackup {
    inner: Mutex<Inner>,
    publisher: Mutex<Option<Publisher>>,
    available: std::sync::atomic::AtomicBool,
}

impl std::fmt::Debug for FakeBackup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeBackup").finish_non_exhaustive()
    }
}

impl Default for FakeBackup {
    fn default() -> Self {
        let inner = Inner { clock: 1_790_000_000, ..Inner::default() };
        Self {
            inner: Mutex::new(inner),
            publisher: Mutex::new(None),
            available: std::sync::atomic::AtomicBool::new(true),
        }
    }
}

fn refused(reason: &'static str) -> Reply {
    Reply::Refused {
        code: "BackupRefused",
        message: reason.to_owned(),
        reason_code: reason,
        details: None,
    }
}

/// The selectors of `count` unavailable files after `after` (a page).
fn missing_page(count: u64, after: &str) -> (Vec<String>, Option<String>) {
    let start = after.strip_prefix("obj-").and_then(|n| n.parse::<u64>().ok()).map_or(1, |n| n + 1);
    let page: Vec<String> =
        (start..=count).take(MISSING_PAGE).map(|n| format!("obj-{n}")).collect();
    let next = (page.len() == MISSING_PAGE).then(|| page.last().cloned()).flatten();
    (page, next)
}

impl FakeBackup {
    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("fake backup")
    }

    /// Installs where job changes go (the daemon-level job stream).
    pub fn set_publisher(&self, publisher: Publisher) {
        *self.publisher.lock().expect("publisher") = Some(publisher);
    }

    /// Makes the daemon one without the job API (`false`), or with it.
    pub fn set_available(&self, available: bool) {
        self.available.store(available, std::sync::atomic::Ordering::SeqCst);
    }

    /// The daemon restarted: it forgets every job (and discards their
    /// staging), as the real one does.
    pub fn forget_jobs(&self) {
        self.inner().jobs.clear();
    }

    /// Declares an account on this device and what a backup of it holds.
    pub fn account(&self, account: &str, holds: Counts) {
        let mut inner = self.inner();
        inner.accounts.insert(account.to_owned());
        inner.holdings.insert(account.to_owned(), holds);
    }

    /// Declares files `account` references without holding them: a backup
    /// fetches them first; `unavailable` of them cannot be fetched.
    pub fn missing_files(&self, account: &str, fetchable: u64, unavailable: u64) {
        self.inner().missing.insert(account.to_owned(), Missing { fetchable, unavailable });
    }

    /// Declares files `account` references here but has not downloaded (a
    /// merge keeps them, still fetchable).
    pub fn local_pending(&self, account: &str, files: u64) {
        self.inner().local_pending.insert(account.to_owned(), files);
    }

    /// Declares a backup file at `path`.
    pub fn archive(&self, path: &str, archive: Archive) {
        self.inner().archives.insert(path.to_owned(), archive);
    }

    /// The archive at `path`, if the fake holds one (one it wrote, too).
    pub fn archive_at(&self, path: &str) -> Option<Archive> {
        self.inner().archives.get(path).cloned()
    }

    /// Makes disk space short (`Some(bytes)` free) or plentiful (`None`).
    pub fn set_free_space(&self, free: Option<u64>) {
        self.inner().free_space = free;
    }

    /// The row of job `id`.
    pub fn row(&self, id: &str) -> Option<Value> {
        self.inner().jobs.get(id).map(|job| job.row(id))
    }

    /// The newest job's id.
    pub fn last(&self) -> Option<String> {
        let inner = self.inner();
        inner.jobs.iter().max_by_key(|(_, j)| j.order).map(|(id, _)| id.clone())
    }

    fn publish(&self, rows: Vec<Value>) {
        if let Some(publish) = self.publisher.lock().expect("publisher").as_ref() {
            for row in rows {
                publish(row);
            }
        }
    }

    /// Answers a job request; `None` for any other method.
    pub fn handle(&self, method: &str, account: Option<&str>, body: &Value) -> Option<Reply> {
        let ours = ATTENDED_METHODS.contains(&method) || method == "account.backup.jobs";
        if !ours {
            return None;
        }
        if !self.available.load(std::sync::atomic::Ordering::SeqCst) {
            let message = format!("unsupported local bus method: {method}");
            return Some(Reply::Error("LocalBus", message));
        }
        let (reply, rows) = {
            let mut inner = self.inner();
            match method {
                "account.backup.jobs" => (Self::jobs(&inner, body), Vec::new()),
                "account.backup.create" | "account.restore.verify" => {
                    Self::start(&mut inner, account, method, body)
                }
                _ => Self::decide(&mut inner, method, body),
            }
        };
        self.publish(rows);
        Some(reply)
    }

    fn jobs(inner: &Inner, body: &Value) -> Reply {
        if let Some(id) = body["job_id"].as_str() {
            let Some(job) = inner.jobs.get(id) else { return refused("unknown_job") };
            if job.error.as_ref().is_none_or(|e| e["code"] != "attachments_unavailable") {
                return refused("staging_expired");
            }
            let count = job.missing.unavailable;
            let (page, next) = missing_page(count, body["missing_after"].as_str().unwrap_or(""));
            return Reply::Ok(json!({
                "job": job.row(id), "unavailable_attachments": page,
                "unavailable_count": count, "unavailable_next_cursor": next,
            }));
        }
        let mut jobs: Vec<(&String, &Job)> = inner.jobs.iter().collect();
        jobs.sort_by_key(|(_, j)| std::cmp::Reverse(j.order));
        let rows: Vec<Value> = jobs.iter().map(|(id, j)| j.row(id)).collect();
        Reply::Ok(json!({ "jobs": rows }))
    }

    fn start(
        inner: &mut Inner,
        account: Option<&str>,
        method: &str,
        body: &Value,
    ) -> (Reply, Vec<Value>) {
        let backup = method == "account.backup.create";
        let account = match (backup, account) {
            (true, None) => return (refused("account_locked"), Vec::new()),
            (_, account) => account.unwrap_or_default().to_owned(),
        };
        let path = body[if backup { "output_path" } else { "input_path" }]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if path.is_empty() {
            return (refused(if backup { "not_writable" } else { "unreadable" }), Vec::new());
        }
        let Some(passphrase) = body["passphrase"].as_str().map(str::to_owned) else {
            return (refused("wrong_passphrase"), Vec::new());
        };
        let allow_missing = match body.get("allow_missing_attachments") {
            None => false,
            Some(Value::Bool(allow)) => *allow,
            Some(_) => return (refused("invalid_attachment_policy"), Vec::new()),
        };
        if inner.jobs.values().any(|j| !j.ended()) {
            return (refused("busy"), Vec::new());
        }
        let found = inner.archives.get(&path).cloned();
        let archive = if backup {
            Archive {
                created_at: inner.clock,
                ..Archive::new(
                    &passphrase,
                    &account,
                    &account,
                    inner.holdings.get(&account).copied().unwrap_or_default(),
                )
            }
        } else {
            found.clone().unwrap_or_else(|| Archive::new("", "", "", Counts::default()))
        };
        inner.next += 1;
        inner.clock += 1;
        let id = format!("job-{}", inner.next);
        let merge = inner.accounts.contains(&archive.account_id);
        let local_pending = if merge {
            inner.local_pending.get(&archive.account_id).copied().unwrap_or(0)
        } else {
            0
        };
        let job = Job {
            kind: if backup { "backup" } else { "restore" },
            path,
            state: if backup { "preparing" } else { "verifying" },
            seq: 1,
            order: inner.next,
            created_at: inner.clock,
            finished_at: None,
            fetched: 0,
            done: 0,
            installed: 0,
            missing: if backup {
                inner.missing.get(&account).copied().unwrap_or_default()
            } else {
                Missing::default()
            },
            allow_missing,
            unreadable: !backup && found.is_none(),
            wrong_passphrase: !backup && found.is_some_and(|a| a.passphrase != passphrase),
            archive,
            merge,
            local_pending,
            error: None,
            checks: None,
        };
        let row = job.row(&id);
        inner.jobs.insert(id, job);
        (Reply::Ok(json!({ "job": row })), vec![row])
    }

    fn decide(inner: &mut Inner, method: &str, body: &Value) -> (Reply, Vec<Value>) {
        let id = body["job_id"].as_str().unwrap_or_default().to_owned();
        let now = inner.clock;
        let Some(job) = inner.jobs.get_mut(&id) else {
            return (refused("unknown_job"), Vec::new());
        };
        match (method, job.state) {
            ("account.restore.install", "verified") => job.state = "installing",
            ("account.backup.cancel", "preparing" | "writing" | "verifying" | "verified") => {
                job.state = "cancelled";
                job.finished_at = Some(now);
            }
            _ => return (refused("invalid_job_phase"), Vec::new()),
        }
        job.seq += 1;
        let row = job.row(&id);
        (Reply::Ok(json!({ "job": row })), vec![row])
    }

    /// Moves every running job one step and publishes the changes.
    pub fn step(&self) {
        let rows = {
            let mut inner = self.inner();
            inner.clock += 1;
            let (now, free) = (inner.clock, inner.free_space);
            let mut rows = Vec::new();
            let mut written = Vec::new();
            for (id, job) in &mut inner.jobs {
                if job.ended() || job.state == "verified" {
                    continue;
                }
                Self::advance(job, now, free);
                job.seq += 1;
                if job.kind == "backup" && job.state == "done" {
                    written.push((job.path.clone(), job.archive.clone()));
                }
                rows.push(job.row(id));
            }
            for (path, archive) in written {
                inner.archives.insert(path, archive);
            }
            rows
        };
        self.publish(rows);
    }

    fn advance(job: &mut Job, now: i64, free: Option<u64>) {
        match job.state {
            "preparing" => Self::prepare(job, now),
            "writing" | "verifying" => Self::read_or_write(job, now, free),
            "installing" => {
                job.installed += 1;
                if job.installed == CATEGORIES.len() {
                    job.state = "done";
                    job.finished_at = Some(now);
                    let mut checks = job.file_checks();
                    checks["local_missing_kept"] = json!(job.local_pending);
                    job.checks = Some(checks);
                }
            }
            _ => {}
        }
    }

    /// Fetches the missing files (two steps when there are any), then
    /// writes, or fails on the files that stay unavailable.
    fn prepare(job: &mut Job, now: i64) {
        let total = job.missing.fetchable + job.missing.unavailable;
        if job.fetched < total {
            job.fetched = (job.fetched + total.div_ceil(2)).min(total);
            return;
        }
        let unavailable = job.missing.unavailable;
        if unavailable > 0 && !job.allow_missing {
            let (page, next) = missing_page(unavailable, "");
            let error = json!({
                "code": "attachments_unavailable", "unavailable_count": unavailable,
                "unavailable_attachments": page, "unavailable_next_cursor": next,
            });
            return job.fail(now, error);
        }
        job.archive.unavailable_at_source = unavailable;
        job.state = "writing";
    }

    fn read_or_write(job: &mut Job, now: i64, free: Option<u64>) {
        let steps = CATEGORIES.len();
        let restore = job.kind == "restore";
        if job.done == 0 {
            if restore && job.unreadable {
                return job.fail(now, json!({ "code": "unreadable" }));
            }
            if restore && job.wrong_passphrase {
                return job.fail(now, json!({ "code": "wrong_passphrase" }));
            }
            let needed = job.archive.counts.archive_bytes();
            if let Some(free) = free.filter(|free| *free < needed) {
                let error = json!({ "code": "insufficient_space",
                                    "needed_bytes": needed, "available_bytes": free });
                return job.fail(now, error);
            }
        }
        job.done += 1;
        let defect = if restore { job.archive.defect } else { None };
        match defect {
            Some(Defect::Tampered) if job.done == steps / 2 => {
                return job.fail(now, json!({ "code": "tampered" }));
            }
            Some(Defect::Truncated) if job.done == steps => {
                return job.fail(now, json!({ "code": "truncated" }));
            }
            Some(Defect::RootConflict) if job.done == steps => {
                return job.fail(now, json!({ "code": "root_conflict" }));
            }
            Some(Defect::IdentityRollback) if job.done == steps && !job.merge => {
                return job.fail(now, json!({ "code": "identity_rollback" }));
            }
            _ => {}
        }
        if job.done < steps {
            return;
        }
        if !restore {
            job.state = "done";
            job.finished_at = Some(now);
            return;
        }
        // Every file is checked; any mismatch, or a file missing that the
        // backup did not record, fails the whole restore.
        job.checks = Some(job.file_checks());
        if job.archive.mismatched > 0 || job.archive.missing > 0 {
            return job.fail(now, json!({ "code": "attachment_mismatch" }));
        }
        job.state = "verified";
    }

    /// Steps until no job runs (a verified restore waits for the user).
    pub fn run_out(&self) {
        for _ in 0..(CATEGORIES.len() * 3) {
            self.step();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(reply: Reply) -> Value {
        match reply {
            Reply::Ok(body) => body,
            other => panic!("expected an answer, got {other:?}"),
        }
    }

    fn reason(reply: Reply) -> &'static str {
        match reply {
            Reply::Refused { reason_code, .. } => reason_code,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    fn counts() -> Counts {
        Counts {
            contacts: 5,
            messages: 10,
            attachments: 3,
            attachment_bytes: 300,
            ..Counts::default()
        }
    }

    #[test]
    fn unavailable_files_fail_a_backup_unless_explicitly_allowed_then_are_recorded() {
        let fake = FakeBackup::default();
        fake.account("a", counts());
        fake.missing_files("a", 2, 70);
        let create = |allow: Option<bool>| {
            let mut body = json!({ "output_path": "/b/x", "passphrase": "p" });
            if let Some(allow) = allow {
                body["allow_missing_attachments"] = json!(allow);
            }
            body
        };
        let started = ok(fake.handle("account.backup.create", Some("a"), &create(None)).unwrap());
        assert_eq!(started["job"]["state"], "preparing");
        fake.step();
        let id = fake.last().unwrap();
        let row = fake.row(&id).unwrap();
        assert_eq!(row["progress"]["category"], "attachments");
        assert_eq!(row["progress"]["items_total"], 72, "fetching every missing file first");
        fake.run_out();
        let failed = fake.row(&id).unwrap();
        assert_eq!(failed["state"], "failed");
        assert_eq!(failed["error"]["code"], "attachments_unavailable");
        assert_eq!(failed["error"]["unavailable_count"], 70);
        assert_eq!(failed["error"]["unavailable_attachments"].as_array().unwrap().len(), 64);
        assert_eq!(failed["error"]["unavailable_next_cursor"], "obj-64");
        let page = ok(fake
            .handle(
                "account.backup.jobs",
                None,
                &json!({ "job_id": id, "missing_after": "obj-64" }),
            )
            .unwrap());
        assert_eq!(page["unavailable_attachments"].as_array().unwrap().len(), 6);
        assert_eq!(page["unavailable_next_cursor"], Value::Null);
        let refused = fake.handle(
            "account.backup.create",
            Some("a"),
            &json!({ "output_path": "/b/x", "passphrase": "p", "allow_missing_attachments": "yes" }),
        );
        assert_eq!(reason(refused.unwrap()), "invalid_attachment_policy");
        ok(fake.handle("account.backup.create", Some("a"), &create(Some(true))).unwrap());
        fake.run_out();
        let done = fake.row(&fake.last().unwrap()).unwrap();
        assert_eq!(done["state"], "done");
        assert_eq!(fake.archive_at("/b/x").unwrap().unavailable_at_source, 70);
    }

    #[test]
    fn a_mismatched_file_fails_verification_and_nothing_installs() {
        let fake = FakeBackup::default();
        fake.archive("/b/x", Archive { mismatched: 1, ..Archive::new("p", "a", "A", counts()) });
        let body = json!({ "input_path": "/b/x", "passphrase": "p" });
        ok(fake.handle("account.restore.verify", None, &body).unwrap());
        fake.run_out();
        let row = fake.row(&fake.last().unwrap()).unwrap();
        assert_eq!(row["state"], "failed");
        assert_eq!(row["error"]["code"], "attachment_mismatch");
        assert_eq!(row["attachments"]["mismatched"], 1);
        let install = json!({ "job_id": fake.last().unwrap() });
        let refusal = fake.handle("account.restore.install", None, &install).unwrap();
        assert_eq!(reason(refusal), "invalid_job_phase");
    }

    #[test]
    fn a_merge_reports_what_it_kept_per_category_and_one_job_runs_at_a_time() {
        let fake = FakeBackup::default();
        fake.account("a", counts());
        fake.local_pending("a", 4);
        fake.archive(
            "/b/x",
            Archive { unavailable_at_source: 2, ..Archive::new("p", "a", "A", counts()) },
        );
        let body = json!({ "input_path": "/b/x", "passphrase": "p" });
        ok(fake.handle("account.restore.verify", None, &body).unwrap());
        assert_eq!(reason(fake.handle("account.restore.verify", None, &body).unwrap()), "busy");
        fake.run_out();
        let id = fake.last().unwrap();
        let verified = fake.row(&id).unwrap();
        assert_eq!(verified["state"], "verified");
        assert_eq!(verified["archive"]["merge"], true);
        assert_eq!(verified["attachments"]["unavailable_at_source"], 2);
        assert_eq!(verified["attachments"]["missing"], 2);
        ok(fake.handle("account.restore.install", None, &json!({ "job_id": id })).unwrap());
        fake.run_out();
        let done = fake.row(&id).unwrap();
        assert_eq!(done["state"], "done");
        assert_eq!(done["kept_newer"]["messages"], 2);
        assert_eq!(done["kept_newer"]["contacts"], 1);
        assert_eq!(done["restored"]["contacts"], 4);
        assert_eq!(done["restored"]["messages"], 8);
        assert_eq!(done["attachments"]["local_missing_kept"], 4);
    }
}
