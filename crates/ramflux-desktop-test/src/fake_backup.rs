// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The fake daemon's full backup and restore: the job API the desktop
//! assumes for BACKUP-01 (`docs/contract.md`, "Full backup and restore").
//!
//! Requests (account scoped):
//!
//! - `account.backup.create {output_path, passphrase}` → `{job}`,
//!   `preparing`;
//! - `account.restore.verify {input_path, passphrase}` → `{job}`,
//!   `verifying`; refused (`reason_code: unreadable`) for a file the fake
//!   does not hold;
//! - `account.restore.install {job_id}` → `{job}`, `installing` (only a
//!   `verified` restore);
//! - `account.backup.cancel {job_id}` → `{job}`, `cancelled` (until a
//!   restore installs);
//! - `account.backup.jobs {}` → `{jobs: [rows]}`, newest first.
//!   Each request but the first two refuses a job already running
//!   (`reason_code: busy`).
//!
//! A job moves one step each time the test calls [`FakeBackup::step`] (so a
//! test or a screenshot can hold it in any phase): a backup writes its
//! categories (identity, contacts, conversations, messages, groups,
//! surfaces, tasks, settings, attachments), a restore reads and verifies
//! them, then installs them. Every change is published as
//! `account.backup.job.changed` through the publisher the test installs.
//!
//! Archives are what the test declares ([`FakeBackup::archive`]): the
//! passphrase that opens one, the account and counts it holds, and a defect
//! (changed or cut short, files whose bytes do not match). A backup the fake
//! writes becomes such an archive. Disk space can be made short
//! ([`FakeBackup::set_free_space`]).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use serde_json::{Value, json};

use crate::fake_bus::Reply;

/// The topic of job changes.
pub const TOPIC: &str = "account.backup.job.changed";

/// Delivers one event: `(account, topic, body)`.
pub type Publisher = Box<dyn Fn(&str, &str, Value) + Send + Sync>;

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

/// What is wrong with an archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Defect {
    /// A chunk was changed: authentication fails at `category`.
    Tampered,
    /// The file ends early: the last chunk is missing.
    Truncated,
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
    /// attachments and the attachments' bytes.
    pub counts: Counts,
    /// What is wrong with it.
    pub defect: Option<Defect>,
    /// Files whose bytes do not match their hash.
    pub mismatched: u64,
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

    fn items(self) -> u64 {
        self.contacts
            + self.conversations
            + self.messages
            + self.groups
            + self.surfaces
            + self.tasks
            + self.attachments
    }

    /// What the archive's size would be (the fake's own measure).
    fn archive_bytes(self) -> u64 {
        self.attachment_bytes + self.items() * 512 + 4096
    }
}

#[derive(Debug, Clone)]
struct Job {
    account: String,
    kind: &'static str,
    path: String,
    state: &'static str,
    seq: u64,
    created_at: i64,
    finished_at: Option<i64>,
    /// Categories done.
    done: usize,
    /// Installing: categories installed.
    installed: usize,
    archive: Archive,
    merge: bool,
    error: Option<Value>,
}

impl Job {
    fn ended(&self) -> bool {
        matches!(self.state, "done" | "failed" | "cancelled")
    }

    fn row(&self, id: &str) -> Value {
        let counts = self.archive.counts;
        let total_bytes = counts.archive_bytes();
        let steps = CATEGORIES.len();
        let (done, category) = match self.state {
            "installing" => (self.installed, CATEGORIES.get(self.installed)),
            "preparing" => (0, None),
            _ => (self.done, CATEGORIES.get(self.done)),
        };
        let fraction = |n: u64| n * u64::try_from(done.min(steps)).unwrap_or(0) / steps as u64;
        let mut row = json!({
            "job_id": id, "kind": self.kind, "state": self.state, "seq": self.seq,
            "path": self.path, "created_at": self.created_at, "finished_at": self.finished_at,
            "progress": {
                "category": if self.ended() { None } else { category },
                "items_done": fraction(counts.items()), "items_total": counts.items(),
                "bytes_done": fraction(total_bytes), "bytes_total": total_bytes,
            },
            "error": self.error,
        });
        let verified = self.kind == "restore"
            && matches!(self.state, "verified" | "installing" | "done" | "cancelled")
            && self.done == steps;
        if (self.kind == "backup" && self.state == "done") || verified {
            row["counts"] = counts.json();
            row["archive_bytes"] = json!(total_bytes);
        }
        if verified {
            row["archive"] = json!({
                "local_account_id": self.archive.account_id,
                "account_label": self.archive.label,
                "created_at": self.archive.created_at,
                "merge": self.merge,
            });
        }
        if self.kind == "restore" && self.state == "done" {
            // Merging: one item of each kind is newer here and kept.
            let kept = if self.merge { 3 } else { 0 };
            let mut restored = counts;
            restored.messages = restored.messages.saturating_sub(kept);
            restored.attachments = counts.attachments - self.archive.mismatched;
            row["restored"] = restored.json();
            row["kept_newer"] = json!(kept);
            row["attachments"] = json!({
                "verified": counts.attachments - self.archive.mismatched,
                "mismatched": self.archive.mismatched, "missing": 0,
            });
        }
        row
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
    /// Free disk space, when short.
    free_space: Option<u64>,
    next: u64,
    clock: i64,
}

type Event = (String, Value);

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

fn refused(reason: &'static str, message: &str, details: Option<Value>) -> Reply {
    Reply::Refused {
        code: "ValidationFailed",
        message: message.to_owned(),
        reason_code: reason,
        details,
    }
}

impl FakeBackup {
    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("fake backup")
    }

    /// Installs where job changes go.
    pub fn set_publisher(&self, publisher: Publisher) {
        *self.publisher.lock().expect("publisher") = Some(publisher);
    }

    /// Makes the daemon one without the job API (`false`), or with it.
    pub fn set_available(&self, available: bool) {
        self.available.store(available, std::sync::atomic::Ordering::SeqCst);
    }

    /// Declares an account on this device and what a backup of it holds.
    pub fn account(&self, account: &str, holds: Counts) {
        let mut inner = self.inner();
        inner.accounts.insert(account.to_owned());
        inner.holdings.insert(account.to_owned(), holds);
    }

    /// Declares a backup file at `path`.
    pub fn archive(&self, path: &str, archive: Archive) {
        self.inner().archives.insert(path.to_owned(), archive);
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
        inner
            .jobs
            .iter()
            .max_by_key(|(id, j)| (j.created_at, (*id).clone()))
            .map(|(i, _)| i.clone())
    }

    fn publish(&self, events: Vec<Event>) {
        if let Some(publish) = self.publisher.lock().expect("publisher").as_ref() {
            for (account, row) in events {
                publish(&account, TOPIC, row);
            }
        }
    }

    /// Answers a job request; `None` for any other method.
    pub fn handle(&self, method: &str, account: Option<&str>, body: &Value) -> Option<Reply> {
        let ours = matches!(
            method,
            "account.backup.create"
                | "account.restore.verify"
                | "account.restore.install"
                | "account.backup.cancel"
                | "account.backup.jobs"
        );
        if !ours {
            return None;
        }
        if !self.available.load(std::sync::atomic::Ordering::SeqCst) {
            let message = format!("unsupported local bus method: {method}");
            return Some(Reply::Error("LocalBus", message));
        }
        let account = account.unwrap_or_default().to_owned();
        let (reply, events) = {
            let mut inner = self.inner();
            match method {
                "account.backup.jobs" => {
                    let mut jobs: Vec<(&String, &Job)> =
                        inner.jobs.iter().filter(|(_, j)| j.account == account).collect();
                    jobs.sort_by_key(|(id, j)| std::cmp::Reverse((j.created_at, (*id).clone())));
                    let rows: Vec<Value> = jobs.iter().map(|(id, j)| j.row(id)).collect();
                    (Reply::Ok(json!({ "jobs": rows })), Vec::new())
                }
                "account.backup.create" | "account.restore.verify" => {
                    Self::start(&mut inner, &account, method, body)
                }
                _ => Self::decide(&mut inner, method, body),
            }
        };
        self.publish(events);
        Some(reply)
    }

    fn start(inner: &mut Inner, account: &str, method: &str, body: &Value) -> (Reply, Vec<Event>) {
        if inner.jobs.values().any(|j| j.account == account && !j.ended()) {
            return (refused("busy", "another backup or restore is running", None), Vec::new());
        }
        let passphrase = body["passphrase"].as_str().unwrap_or_default().to_owned();
        let backup = method == "account.backup.create";
        let path = body[if backup { "output_path" } else { "input_path" }]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if passphrase.is_empty() || path.is_empty() {
            return (
                Reply::Error("ValidationFailed", "missing path or passphrase".into()),
                Vec::new(),
            );
        }
        let archive = if backup {
            Archive {
                passphrase,
                account_id: account.to_owned(),
                label: account.to_owned(),
                created_at: inner.clock,
                counts: inner.holdings.get(account).copied().unwrap_or_default(),
                defect: None,
                mismatched: 0,
            }
        } else {
            match inner.archives.get(&path) {
                Some(archive) => archive.clone(),
                None => {
                    return (
                        refused("unreadable", "cannot read the backup file", None),
                        Vec::new(),
                    );
                }
            }
        };
        inner.next += 1;
        inner.clock += 1;
        let id = format!("job-{}", inner.next);
        let merge = inner.accounts.contains(&archive.account_id);
        let mut job = Job {
            account: account.to_owned(),
            kind: if backup { "backup" } else { "restore" },
            path,
            state: if backup { "preparing" } else { "verifying" },
            seq: 1,
            created_at: inner.clock,
            finished_at: None,
            done: 0,
            installed: 0,
            archive,
            merge,
            error: None,
        };
        // A wrong passphrase is found once the archive's key is unwrapped.
        if !backup && body["passphrase"] != job.archive.passphrase.as_str() {
            job.archive.counts = Counts::default();
            job.state = "failed";
            job.finished_at = Some(inner.clock);
            job.error = Some(json!({ "code": "wrong_passphrase" }));
        }
        let row = job.row(&id);
        inner.jobs.insert(id, job);
        (Reply::Ok(json!({ "job": row })), vec![(account.to_owned(), row)])
    }

    fn decide(inner: &mut Inner, method: &str, body: &Value) -> (Reply, Vec<Event>) {
        let id = body["job_id"].as_str().unwrap_or_default().to_owned();
        let now = inner.clock;
        let Some(job) = inner.jobs.get_mut(&id) else {
            return (Reply::Error("NotFound", "no such job".into()), Vec::new());
        };
        match (method, job.state) {
            ("account.restore.install", "verified") => job.state = "installing",
            ("account.backup.cancel", "preparing" | "writing" | "verifying" | "verified") => {
                job.state = "cancelled";
                job.finished_at = Some(now);
            }
            _ => return (refused("state", "the job cannot do this now", None), Vec::new()),
        }
        job.seq += 1;
        let row = job.row(&id);
        (Reply::Ok(json!({ "job": row })), vec![(job.account.clone(), row)])
    }

    /// Moves every running job one step and publishes the changes.
    pub fn step(&self) {
        let events = {
            let mut inner = self.inner();
            inner.clock += 1;
            let (now, free) = (inner.clock, inner.free_space);
            let mut events = Vec::new();
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
                events.push((job.account.clone(), job.row(id)));
            }
            for (path, archive) in written {
                inner.archives.insert(path, archive);
            }
            events
        };
        self.publish(events);
    }

    fn advance(job: &mut Job, now: i64, free: Option<u64>) {
        let steps = CATEGORIES.len();
        let needed = job.archive.counts.archive_bytes();
        let fail = |job: &mut Job, error: Value| {
            job.state = "failed";
            job.finished_at = Some(now);
            job.error = Some(error);
        };
        match job.state {
            "preparing" => job.state = "writing",
            "writing" | "verifying" => {
                if job.done == 0
                    && let Some(free) = free.filter(|free| *free < needed)
                {
                    let error = json!({ "code": "insufficient_space",
                                        "needed_bytes": needed, "available_bytes": free });
                    return fail(job, error);
                }
                job.done += 1;
                let defect = if job.kind == "restore" { job.archive.defect } else { None };
                match defect {
                    Some(Defect::Tampered) if job.done == steps / 2 => {
                        return fail(job, json!({ "code": "tampered" }));
                    }
                    Some(Defect::Truncated) if job.done == steps => {
                        return fail(job, json!({ "code": "truncated" }));
                    }
                    _ => {}
                }
                if job.done == steps {
                    if job.kind == "backup" {
                        job.state = "done";
                        job.finished_at = Some(now);
                    } else {
                        job.state = "verified";
                    }
                }
            }
            "installing" => {
                job.installed += 1;
                if job.installed == steps {
                    job.state = "done";
                    job.finished_at = Some(now);
                }
            }
            _ => {}
        }
    }

    /// Steps until no job runs (a verified restore waits for the user).
    pub fn run_out(&self) {
        for _ in 0..(CATEGORIES.len() * 3) {
            self.step();
        }
    }
}
