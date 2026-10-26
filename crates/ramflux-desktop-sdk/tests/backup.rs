// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Full backup and restore through the SDK source and a fake daemon with
//! the BACKUP-01 job API as merged (product `docs/account_backup.md`): the
//! source opens the daemon-level job stream as an attended frontend and
//! sends every job request on that connection (the daemon takes them from
//! no other); a backup reaches the daemon as `account.backup.create` in the
//! account's scope, strict about unavailable files unless explicitly
//! allowed, a restore as a daemon-level `account.restore.verify`, both with
//! the passphrase as a secret; job rows (the job stream, answers, reads)
//! drive the store's job state machine through fetching, progress,
//! verification, the user's review, installation and the final per-category
//! counts and file check; typed failures arrive typed; and a daemon without
//! the job API declares every backup method unsupported. Every row the fake
//! writes is checked against the product's own job type.

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Weak};

use common::*;
use ramflux_desktop_core::{
    AccountId, BackupCategory, BackupFailure, BackupJob, BackupJobKind, BackupPhase, ContextId,
    Intent, IntentError, IntentKind, SecretInput, SecureSessions, Source, Store, SyncMode,
    Timestamp, handle_intent, methods, selectors,
};
use ramflux_desktop_sdk::capabilities;
use ramflux_desktop_test::fake_backup::{Archive, Counts, Defect, FakeBackup, TOPIC};
use serde_json::Value;

const ACCOUNT: &str = "acct-main";
const PASSPHRASE: &str = "river stone lantern harbor";

struct World {
    fake: Arc<FakeBus>,
    backup: Arc<FakeBackup>,
    path: PathBuf,
}

fn holdings() -> Counts {
    Counts {
        contacts: 1_200,
        conversations: 240,
        messages: 12_000,
        groups: 210,
        surfaces: 9,
        tasks: 4,
        attachments: 320,
        attachment_bytes: 1_300_000_000,
    }
}

fn archive(defect: Option<Defect>, mismatched: u64) -> Archive {
    Archive { defect, mismatched, ..Archive::new(PASSPHRASE, ACCOUNT, "Personal", holdings()) }
}

/// Checks a row against the product's own job type (its fields and their
/// shapes).
fn product(row: Value) -> ramflux_sdk::backup::BackupJob {
    serde_json::from_value(row).expect("a product job row")
}

impl World {
    fn start(name: &str, stream: bool) -> Self {
        let path = socket_path(&format!("backup-{name}"));
        let backup = Arc::new(FakeBackup::default());
        backup.account(ACCOUNT, holdings());
        let handler = backup.clone();
        let fake = Arc::new(FakeBus::start(&path, move |method, account, body| {
            handler.handle(method, account, body).unwrap_or_else(|| fixture(method))
        }));
        if stream {
            fake.enable_stream();
        }
        let bus: Weak<FakeBus> = Arc::downgrade(&fake);
        backup.set_publisher(Box::new(move |row| {
            product(row.clone());
            if let Some(bus) = bus.upgrade() {
                bus.publish_job(row);
            }
        }));
        Self { fake, backup, path }
    }

    fn requests(&self, method: &str) -> Vec<ramflux_desktop_test::fake_bus::Seen> {
        self.fake.seen().into_iter().filter(|s| s.method == method).collect()
    }

    /// The connections that opened the daemon-level job stream as an
    /// attended frontend.
    fn job_connections(&self) -> Vec<usize> {
        self.fake
            .seen()
            .into_iter()
            .filter(|s| {
                s.method == "bus.subscribe"
                    && s.account.is_none()
                    && s.body["topics"] == serde_json::json!([TOPIC])
                    && s.body["attended_frontend"] == true
            })
            .map(|s| s.connection)
            .collect()
    }

    /// Checks that `method` went on an attended job connection.
    fn attended(&self, method: &str) -> ramflux_desktop_test::fake_bus::Seen {
        let sent = self.requests(method).pop().unwrap_or_else(|| panic!("{method} sent"));
        assert!(
            self.job_connections().contains(&sent.connection),
            "{method} travels on the attended job connection"
        );
        sent
    }

    fn stop(self) {
        if let Ok(fake) = Arc::try_unwrap(self.fake) {
            fake.shutdown();
        }
    }
}

fn live(world: &World, stream: bool) -> Fed {
    let hub = ramflux_desktop_sdk::SdkHub::start(config(&world.path)).expect("bridge");
    let mut account = Fed::new(hub.account_source(AccountId::new(ACCOUNT)));
    let mode = if stream { SyncMode::Stream } else { SyncMode::Polling };
    account.until("the account is live", |s, _| {
        s.context(&ContextId::new("dm-ana")).is_some()
            && s.sync_status().is_some_and(|sync| sync.mode == mode)
            && s.unsupported_reason(methods::BACKUP_CREATE).is_none()
    });
    account.until("the job stream is open", |_, _| world.fake.job_streaming());
    account
}

fn create(path: &str, allow_missing_attachments: bool) -> IntentKind {
    IntentKind::CreateBackup {
        account_id: AccountId::new(ACCOUNT),
        path: path.into(),
        secret: SecretInput::new(PASSPHRASE),
        allow_missing_attachments,
    }
}

fn dispatch(account: &mut Fed, kind: IntentKind) {
    let effects =
        handle_intent(&account.store, Intent::human(kind), Timestamp(0)).expect("allowed");
    for effect in effects {
        account.source.submit(effect);
    }
}

fn job(store: &Store) -> Option<&BackupJob> {
    selectors::latest_backup_job(store)
}

fn phase_is(phase: BackupPhase) -> impl Fn(&Store, &[ramflux_desktop_core::EventEnvelope]) -> bool {
    move |store, _| job(store).is_some_and(|j| j.phase == phase)
}

fn verify(account: &mut Fed, path: &str, passphrase: &str) {
    dispatch(
        account,
        IntentKind::VerifyRestore { path: path.into(), secret: SecretInput::new(passphrase) },
    );
}

#[test]
fn a_full_backup_writes_everything_with_progress_and_reports_its_counts() {
    let world = World::start("create", true);
    let mut account = live(&world, true);
    dispatch(&mut account, create("/backups/personal.rfbackup", false));
    account.until("preparing", phase_is(BackupPhase::Preparing));
    let sent = world.attended("account.backup.create");
    assert_eq!(sent.account.as_deref(), Some(ACCOUNT));
    assert_eq!(sent.body["output_path"], "/backups/personal.rfbackup");
    assert_eq!(sent.body["allow_missing_attachments"], false, "strict unless allowed");
    assert_eq!(sent.body["passphrase"], PASSPHRASE, "the secret reaches the daemon");
    assert!(sent.operation_id.is_some(), "a job start is a mutation");
    // The passphrase never enters the store or an event.
    assert!(!format!("{:?}", account.envelopes).contains(PASSPHRASE));
    // One job at a time.
    assert_eq!(
        handle_intent(&account.store, Intent::human(create("/backups/again", false)), Timestamp(0))
            .unwrap_err(),
        IntentError::BackupInProgress
    );

    // Preparing, then identity, contacts and conversations: messages next.
    for _ in 0..4 {
        world.backup.step();
    }
    account.until("writing messages", |s, _| {
        job(s).is_some_and(|j| {
            j.phase == BackupPhase::Writing && j.progress.category == BackupCategory::Messages
        })
    });
    let progress = job(&account.store).unwrap().progress;
    assert!(progress.bytes_done > 0 && progress.bytes_total > Some(progress.bytes_done));
    assert_eq!(job(&account.store).unwrap().summary, None, "nothing written yet");
    let fraction = progress.fraction().expect("known totals");
    assert!(fraction > 0.0 && fraction < 1.0);

    world.backup.run_out();
    account.until("done", phase_is(BackupPhase::Done));
    let done = job(&account.store).unwrap();
    let summary = done.summary.as_ref().expect("summary");
    assert_eq!(summary.counts.messages, 12_000, "no count limits");
    assert_eq!(summary.counts.contacts, 1_200);
    assert_eq!(summary.counts.groups, 210);
    assert_eq!(summary.counts.attachments, 320);
    assert_eq!(summary.counts.attachment_bytes, 1_300_000_000);
    assert!(summary.file_bytes > summary.counts.attachment_bytes);
    assert!(done.finished_at.is_some());
    world.stop();
}

#[test]
fn a_backup_fetches_missing_files_first_and_records_unavailable_ones_only_when_allowed() {
    let world = World::start("missing", true);
    world.backup.missing_files(ACCOUNT, 6, 70);
    let mut account = live(&world, true);
    dispatch(&mut account, create("/backups/strict.rfbackup", false));
    account.until("preparing", phase_is(BackupPhase::Preparing));
    world.backup.step();
    account.until("fetching", |s, _| {
        job(s).is_some_and(|j| j.progress.is_fetching(j.phase) && j.progress.items_done > 0)
    });
    assert_eq!(job(&account.store).unwrap().progress.items_total, Some(76));
    world.backup.run_out();
    account.until("failed", phase_is(BackupPhase::Failed));
    let Some(BackupFailure::AttachmentsUnavailable { count, selectors, next_cursor }) =
        job(&account.store).unwrap().failure.clone()
    else {
        panic!("expected unavailable files");
    };
    assert_eq!(count, 70, "every unavailable file is counted");
    assert_eq!(selectors.len(), 64, "the first page");
    assert_eq!(next_cursor.as_deref(), Some("obj-64"));
    assert!(world.backup.archive_at("/backups/strict.rfbackup").is_none(), "nothing written");

    // Explicitly allowed: the files are recorded as unavailable at the source.
    dispatch(&mut account, create("/backups/allowed.rfbackup", true));
    account.until("preparing again", |s, _| {
        s.backup_jobs().filter(|j| j.kind == BackupJobKind::Backup).count() == 2
            && job(s).is_some_and(|j| j.phase == BackupPhase::Preparing)
    });
    assert_eq!(world.attended("account.backup.create").body["allow_missing_attachments"], true);
    world.backup.run_out();
    account.until("done", phase_is(BackupPhase::Done));
    let written = world.backup.archive_at("/backups/allowed.rfbackup").expect("written");
    assert_eq!(written.unavailable_at_source, 70);
    world.stop();
}

#[test]
fn a_restore_verifies_waits_for_the_user_then_merges_and_checks_every_file() {
    let world = World::start("restore", true);
    let recorded = Archive { unavailable_at_source: 3, ..archive(None, 0) };
    world.backup.archive("/b/personal.rfbackup", recorded);
    world.backup.local_pending(ACCOUNT, 5);
    let mut account = live(&world, true);
    verify(&mut account, "/b/personal.rfbackup", PASSPHRASE);
    account.until("verifying", phase_is(BackupPhase::Verifying));
    let sent = world.attended("account.restore.verify");
    assert_eq!(sent.account, None, "a restore is daemon-level");
    assert_eq!(sent.body["input_path"], "/b/personal.rfbackup");
    assert_eq!(sent.body["passphrase"], PASSPHRASE);
    let id = job(&account.store).unwrap().id.clone();
    // Nothing installs before the archive verified completely.
    assert_eq!(
        handle_intent(
            &account.store,
            Intent::human(IntentKind::InstallRestore { job_id: id.clone() }),
            Timestamp(0)
        )
        .unwrap_err(),
        IntentError::BackupJobPhase(id.clone())
    );

    world.backup.run_out();
    account.until("verified", phase_is(BackupPhase::Verified));
    let verified = job(&account.store).unwrap();
    let preview = verified.preview.as_ref().expect("what it holds");
    assert!(preview.merge, "the account is already on this device");
    assert_eq!(preview.account_label.as_deref(), Some("Personal"));
    assert_eq!(preview.counts.messages, 12_000);
    assert_eq!(preview.created_at, Timestamp(1_789_000_000_000));
    assert_eq!(preview.attachments.verified, 320, "every file it holds matched");
    assert_eq!(preview.attachments.unavailable_at_source, 3);
    assert!(preview.attachments.all_verified());
    // It waits for the user.
    world.backup.run_out();
    account.settle(200);
    assert_eq!(job(&account.store).unwrap().phase, BackupPhase::Verified);

    let reads_before = world.requests("contact.list").len();
    dispatch(&mut account, IntentKind::InstallRestore { job_id: id.clone() });
    account.until("installing", phase_is(BackupPhase::Installing));
    assert!(
        handle_intent(
            &account.store,
            Intent::human(IntentKind::CancelBackupJob { job_id: id.clone() }),
            Timestamp(0)
        )
        .is_err(),
        "installing cannot be cancelled"
    );
    world.backup.run_out();
    account.until("restored", phase_is(BackupPhase::Done));
    let report = job(&account.store).unwrap().report.clone().expect("report");
    assert_eq!(report.kept_newer.messages, 2, "newer messages on this device are kept");
    assert_eq!(report.kept_newer.contacts, 1, "per category");
    assert_eq!(report.restored.messages, 11_998);
    assert_eq!(report.restored.contacts, 1_199);
    assert_eq!(report.attachments.verified, 320);
    assert_eq!(report.attachments.mismatched, 0);
    assert_eq!(report.attachments.unavailable_at_source, 3);
    assert_eq!(report.attachments.local_missing_kept, 5);
    assert!(report.attachments.all_verified());
    // What it installed is read again.
    account.until("the account read again", |_, _| {
        world.requests("contact.list").len() > reads_before
    });
    let install = world.attended("account.restore.install");
    assert_eq!((install.account, &install.body["job_id"]), (None, &Value::from(id.as_str())));
    world.stop();
}

#[test]
fn a_file_that_does_not_match_fails_the_restore_and_nothing_installs() {
    let world = World::start("mismatch", true);
    world.backup.archive("/b/damaged", archive(None, 2));
    let mut account = live(&world, true);
    verify(&mut account, "/b/damaged", PASSPHRASE);
    account.until("verifying", phase_is(BackupPhase::Verifying));
    world.backup.run_out();
    account.until("failed", phase_is(BackupPhase::Failed));
    let failed = job(&account.store).unwrap();
    let Some(BackupFailure::AttachmentMismatch { checks: Some(checks) }) = &failed.failure else {
        panic!("expected a file mismatch, got {:?}", failed.failure);
    };
    assert_eq!((checks.verified, checks.mismatched), (318, 2));
    assert!(!checks.all_verified());
    assert!(failed.preview.is_none(), "nothing to review");
    let id = failed.id.clone();
    assert_eq!(
        handle_intent(
            &account.store,
            Intent::human(IntentKind::InstallRestore { job_id: id.clone() }),
            Timestamp(0)
        )
        .unwrap_err(),
        IntentError::BackupJobPhase(id)
    );
    assert!(world.requests("account.restore.install").is_empty());
    world.stop();
}

#[test]
fn restore_failures_arrive_typed() {
    let world = World::start("failures", true);
    world.backup.archive("/b/changed", archive(Some(Defect::Tampered), 0));
    world.backup.archive("/b/short", archive(Some(Defect::Truncated), 0));
    world.backup.archive("/b/good", archive(None, 0));
    world.backup.archive("/b/other-root", archive(Some(Defect::RootConflict), 0));
    let mut account = live(&world, true);

    let failure_of = |account: &Fed| job(&account.store).and_then(|j| j.failure.clone());
    verify(&mut account, "/b/good", "wrong words");
    account.until("verifying", phase_is(BackupPhase::Verifying));
    world.backup.step();
    account.until("wrong passphrase", phase_is(BackupPhase::Failed));
    assert_eq!(failure_of(&account), Some(BackupFailure::WrongPassphrase));

    verify(&mut account, "/b/changed", PASSPHRASE);
    account.until("verifying", phase_is(BackupPhase::Verifying));
    world.backup.run_out();
    account.until("tampered", phase_is(BackupPhase::Failed));
    assert_eq!(failure_of(&account), Some(BackupFailure::Tampered));

    verify(&mut account, "/b/short", PASSPHRASE);
    account.until("verifying", phase_is(BackupPhase::Verifying));
    world.backup.run_out();
    account.until("truncated", phase_is(BackupPhase::Failed));
    assert_eq!(failure_of(&account), Some(BackupFailure::Truncated));

    world.backup.set_free_space(Some(1_000));
    verify(&mut account, "/b/good", PASSPHRASE);
    account.until("verifying", phase_is(BackupPhase::Verifying));
    world.backup.step();
    account.until("no space", phase_is(BackupPhase::Failed));
    let Some(BackupFailure::InsufficientSpace { needed, available }) = failure_of(&account) else {
        panic!("expected insufficient space");
    };
    assert_eq!(available, Some(1_000));
    assert!(needed > Some(1_300_000_000));
    world.backup.set_free_space(None);

    verify(&mut account, "/b/other-root", PASSPHRASE);
    account.until("verifying", phase_is(BackupPhase::Verifying));
    world.backup.run_out();
    account.until("root conflict", phase_is(BackupPhase::Failed));
    assert_eq!(failure_of(&account), Some(BackupFailure::RootConflict));

    // A file the daemon cannot read fails the job, typed.
    verify(&mut account, "/b/missing", PASSPHRASE);
    account.until("verifying", phase_is(BackupPhase::Verifying));
    world.backup.step();
    account.until("unreadable", phase_is(BackupPhase::Failed));
    assert_eq!(failure_of(&account), Some(BackupFailure::Unreadable));
    world.stop();
}

#[test]
fn a_job_request_the_daemon_refuses_arrives_typed() {
    let world = World::start("refused", true);
    world.backup.archive("/b/good", archive(None, 0));
    world.backup.archive("/b/other", archive(None, 0));
    let mut account = live(&world, true);
    // Another frontend's restore runs, and this source has not heard of it
    // (its rows are not published): the daemon runs one job at a time.
    world.backup.set_publisher(Box::new(|_| {}));
    let body = serde_json::json!({ "input_path": "/b/other", "passphrase": PASSPHRASE });
    world.backup.handle("account.restore.verify", None, &body).expect("started");
    let failures = account.store.failures_total();
    verify(&mut account, "/b/good", PASSPHRASE);
    account.until("refused", |s, _| s.failures_total() > failures);
    let refusal = account.store.failures().last().unwrap();
    assert_eq!(refusal.method, methods::RESTORE_VERIFY);
    assert_eq!(refusal.message, "backend.backup.busy");
    world.attended("account.restore.verify");
    world.stop();
}

#[test]
fn the_job_stream_resumes_from_its_cursor_after_it_ends() {
    let world = World::start("resume", true);
    world.backup.archive("/b/good", archive(None, 0));
    let mut account = live(&world, true);
    verify(&mut account, "/b/good", PASSPHRASE);
    account.until("verifying", phase_is(BackupPhase::Verifying));
    let opened = world.job_connections().len();
    // The daemon stops the stream (the source fell behind); the job goes on.
    world.fake.end_job_streams(true);
    world.backup.run_out();
    account.until("verified, after the stream opened again", phase_is(BackupPhase::Verified));
    let resumed = world.requests("bus.subscribe");
    let last = resumed.iter().rfind(|s| s.account.is_none()).expect("subscribed again");
    assert!(world.job_connections().len() > opened, "a new attended connection");
    assert!(last.body["since_seq"].as_u64().is_some(), "from the last cursor seen");
    world.stop();
}

#[test]
fn a_job_running_when_the_daemon_restarted_ends_interrupted_and_the_next_may_start() {
    let world = World::start("restart", true);
    world.backup.archive("/b/good", archive(None, 0));
    let mut account = live(&world, true);
    verify(&mut account, "/b/good", PASSPHRASE);
    account.until("verifying", phase_is(BackupPhase::Verifying));
    world.backup.step();
    account.until("progress", |s, _| job(s).is_some_and(|j| j.progress.bytes_done > 0));
    let id = job(&account.store).unwrap().id.clone();
    let opened = world.job_connections().len();
    // The daemon restarts: its jobs (in memory only) and its job stream are
    // gone. The job link opens again from its cursor, which the new daemon
    // does not cover, and the listing it reads then lacks the job.
    world.backup.forget_jobs();
    world.fake.restart_jobs();
    account.until("interrupted", |s, _| {
        s.backup_job(&id).is_some_and(|j| j.phase == BackupPhase::Failed)
    });
    let lost = account.store.backup_job(&id).unwrap();
    assert_eq!(lost.failure, Some(BackupFailure::Interrupted));
    assert!(selectors::running_backup_job(&account.store).is_none(), "nothing runs");
    assert!(world.job_connections().len() > opened, "a new attended connection");
    // The next job may start, on the new daemon.
    verify(&mut account, "/b/good", PASSPHRASE);
    account.until("verifying again", |s, _| {
        job(s).is_some_and(|j| j.id != id && j.phase == BackupPhase::Verifying)
    });
    world.backup.run_out();
    account.until("verified", phase_is(BackupPhase::Verified));
    world.stop();
}

#[test]
fn a_job_known_only_from_the_listing_is_interrupted_when_the_daemon_restarts() {
    let world = World::start("listing-only", true);
    world.backup.archive("/b/good", archive(None, 0));
    let mut account = live(&world, true);
    // The job link is down (its connection broke) while another frontend
    // starts a restore: its rows go by. The link opens again from the head
    // and reads the jobs: the restore is known from the listing only, no
    // event of it ever arrived here.
    let opened = world.job_connections().len();
    world.fake.end_job_streams(false);
    let body = serde_json::json!({ "input_path": "/b/good", "passphrase": PASSPHRASE });
    world.backup.handle("account.restore.verify", None, &body).expect("started elsewhere");
    account.until("the job listed after the link opened again", |s, _| {
        job(s).is_some_and(|j| j.phase == BackupPhase::Verifying)
    });
    assert!(world.job_connections().len() > opened);
    let id = job(&account.store).unwrap().id.clone();
    assert_eq!(
        handle_intent(&account.store, Intent::human(create("/b/again", false)), Timestamp(0))
            .unwrap_err(),
        IntentError::BackupInProgress
    );
    // The daemon restarts. The link resumes from the stream head it was
    // told on subscribing, which the new daemon does not cover: the jobs
    // are read again and the one no longer listed ended with the daemon.
    world.backup.forget_jobs();
    world.fake.restart_jobs();
    account.until("interrupted", |s, _| {
        s.backup_job(&id).is_some_and(|j| j.phase == BackupPhase::Failed)
    });
    assert_eq!(account.store.backup_job(&id).unwrap().failure, Some(BackupFailure::Interrupted));
    assert!(selectors::running_backup_job(&account.store).is_none());
    verify(&mut account, "/b/good", PASSPHRASE);
    account.until("the next job", |s, _| {
        job(s).is_some_and(|j| j.id != id && j.phase == BackupPhase::Verifying)
    });
    world.stop();
}

#[test]
fn secure_sessions_are_reestablished_after_a_restore_until_the_daemon_says_ready() {
    let world = World::start("sessions", true);
    let mut account = live(&world, true);
    let me = AccountId::new(ACCOUNT);
    let sessions = |s: &Store| s.account(&me).map(|a| a.secure_sessions);
    assert_eq!(sessions(&account.store), Some(SecureSessions::Ready));
    // The daemon installed a restore into this account: the account stream
    // says so, and that the device activates itself again before sending.
    world.fake.publish(
        ACCOUNT,
        "account.restored",
        serde_json::json!({ "local_account_id": ACCOUNT, "job_id": "j-x",
                            "secure_sessions": "reestablishing" }),
    );
    account.until("re-establishing", |s, _| sessions(s) == Some(SecureSessions::Reestablishing));
    // Activation done: the daemon announces the sessions ready.
    world.fake.publish(
        ACCOUNT,
        "account.secure_sessions.changed",
        serde_json::json!({ "local_account_id": ACCOUNT, "secure_sessions": "ready" }),
    );
    account.until("ready", |s, _| sessions(s) == Some(SecureSessions::Ready));
    world.stop();
}

#[test]
fn a_restore_is_cancelled_before_it_installs_and_jobs_come_back_after_a_reload() {
    let world = World::start("cancel", false);
    world.backup.archive("/b/good", archive(None, 0));
    let mut account = live(&world, false);
    verify(&mut account, "/b/good", PASSPHRASE);
    account.until("verifying", phase_is(BackupPhase::Verifying));
    // Without the account stream, the job stream still follows the job.
    world.backup.step();
    account.until("progress by polling", |s, _| {
        job(s).is_some_and(|j| j.progress.items_done > 0 || j.progress.bytes_done > 0)
    });
    let id = job(&account.store).unwrap().id.clone();
    dispatch(&mut account, IntentKind::CancelBackupJob { job_id: id.clone() });
    account.until("cancelled", phase_is(BackupPhase::Cancelled));
    assert_eq!(world.attended("account.backup.cancel").body["job_id"], id.as_str());

    // A fresh source reads the jobs with its snapshot.
    let mut again = live(&world, false);
    again.until("the job", |s, _| s.backup_job(&id).is_some());
    let kept = again.store.backup_job(&id).unwrap();
    assert_eq!((kept.kind, kept.phase), (BackupJobKind::Restore, BackupPhase::Cancelled));
    world.stop();
}

#[test]
fn a_daemon_without_the_job_api_declares_backup_unsupported() {
    let world = World::start("absent", true);
    world.backup.set_available(false);
    let hub = ramflux_desktop_sdk::SdkHub::start(config(&world.path)).expect("bridge");
    let mut account = Fed::new(hub.account_source(AccountId::new(ACCOUNT)));
    account.until("the account is live", |s, _| {
        s.sync_status().is_some_and(|sync| sync.mode == SyncMode::Stream)
    });
    for method in capabilities::BACKUP_METHODS {
        assert_eq!(
            account.store.unsupported_reason(method),
            Some(capabilities::reason::FULL_BACKUP),
            "{method}"
        );
    }
    let refused = handle_intent(&account.store, Intent::human(create("/b/x", false)), Timestamp(0));
    assert!(matches!(refused, Err(IntentError::UnsupportedByBackend { .. })));
    assert!(world.requests("account.backup.create").is_empty());
    assert!(world.job_connections().is_empty(), "no job stream without the API");
    world.stop();
}
