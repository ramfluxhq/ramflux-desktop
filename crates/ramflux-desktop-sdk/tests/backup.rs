// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Full backup and restore through the SDK source and a fake daemon with
//! the assumed BACKUP-01 job API: a backup and a restore reach the daemon
//! as `account.backup.create` / `account.restore.verify` with the
//! passphrase as a secret, in the account's scope; job changes (stream, or
//! polls while a job runs) drive the store's job state machine through
//! progress, verification, the user's review, installation and the final
//! counts and attachment check; typed failures (wrong passphrase, a changed
//! or cut-short file, no disk space, an unreadable file) arrive typed; and
//! a daemon without the job API declares every backup method unsupported.

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Weak};

use common::*;
use ramflux_desktop_core::{
    AccountId, BackupCategory, BackupFailure, BackupJob, BackupJobKind, BackupPhase, ContextId,
    Intent, IntentError, IntentKind, SecretInput, Source, Store, SyncMode, Timestamp,
    handle_intent, methods, selectors,
};
use ramflux_desktop_sdk::capabilities;
use ramflux_desktop_test::fake_backup::{Archive, Counts, Defect, FakeBackup};

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
    Archive {
        passphrase: PASSPHRASE.to_owned(),
        account_id: ACCOUNT.to_owned(),
        label: "Personal".to_owned(),
        created_at: 1_789_000_000,
        counts: holdings(),
        defect,
        mismatched,
    }
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
        backup.set_publisher(Box::new(move |account, topic, body| {
            if let Some(bus) = bus.upgrade() {
                bus.publish(account, topic, body.clone());
                bus.push(account, topic, body);
            }
        }));
        Self { fake, backup, path }
    }

    fn requests(&self, method: &str) -> Vec<ramflux_desktop_test::fake_bus::Seen> {
        self.fake.seen().into_iter().filter(|s| s.method == method).collect()
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
    account
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
    dispatch(
        &mut account,
        IntentKind::CreateBackup {
            account_id: AccountId::new(ACCOUNT),
            path: "/backups/personal.rfbackup".into(),
            secret: SecretInput::new(PASSPHRASE),
        },
    );
    account.until("preparing", phase_is(BackupPhase::Preparing));
    let sent = &world.requests("account.backup.create")[0];
    assert_eq!(sent.account.as_deref(), Some(ACCOUNT));
    assert_eq!(sent.body["output_path"], "/backups/personal.rfbackup");
    assert_eq!(sent.body["passphrase"], PASSPHRASE, "the secret reaches the daemon");
    assert!(sent.operation_id.is_some(), "a job start is a mutation");
    // The passphrase never enters the store or an event.
    assert!(!format!("{:?}", account.envelopes).contains(PASSPHRASE));
    // One job at a time.
    let again = IntentKind::CreateBackup {
        account_id: AccountId::new(ACCOUNT),
        path: "/backups/again".into(),
        secret: SecretInput::new(PASSPHRASE),
    };
    assert_eq!(
        handle_intent(&account.store, Intent::human(again), Timestamp(0)).unwrap_err(),
        IntentError::BackupInProgress
    );

    // Preparing, then identity, contacts and conversations: messages next.
    for _ in 0..4 {
        world.backup.step();
    }
    account.until("writing messages", |s, _| {
        job(s).is_some_and(|j| {
            j.phase == BackupPhase::Writing && j.progress.category == Some(BackupCategory::Messages)
        })
    });
    let progress = job(&account.store).unwrap().progress;
    assert_eq!(progress.items_total, Some(13_983));
    assert!(progress.bytes_done > 0 && progress.bytes_total > Some(progress.bytes_done));
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
fn a_restore_verifies_waits_for_the_user_then_merges_and_checks_every_file() {
    let world = World::start("restore", true);
    world.backup.archive("/b/personal.rfbackup", archive(None, 2));
    let mut account = live(&world, true);
    verify(&mut account, "/b/personal.rfbackup", PASSPHRASE);
    account.until("verifying", phase_is(BackupPhase::Verifying));
    let sent = &world.requests("account.restore.verify")[0];
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
    assert_eq!(report.kept_newer, 3, "newer items on this device are kept");
    assert_eq!(report.restored.messages, 11_997);
    assert_eq!(report.attachments.verified, 318);
    assert_eq!(report.attachments.mismatched, 2);
    assert!(!report.attachments.all_verified());
    // What it installed is read again.
    account.until("the account read again", |_, _| {
        world.requests("contact.list").len() > reads_before
    });
    assert_eq!(world.requests("account.restore.install")[0].body["job_id"], id.as_str());
    world.stop();
}

#[test]
fn restore_failures_arrive_typed() {
    let world = World::start("failures", true);
    world.backup.archive("/b/changed", archive(Some(Defect::Tampered), 0));
    world.backup.archive("/b/short", archive(Some(Defect::Truncated), 0));
    world.backup.archive("/b/good", archive(None, 0));
    let mut account = live(&world, true);

    let failure_of = |account: &Fed| job(&account.store).and_then(|j| j.failure.clone());
    verify(&mut account, "/b/good", "wrong words");
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

    // A file the daemon cannot read is refused at once, typed.
    let failures = account.store.failures_total();
    verify(&mut account, "/b/missing", PASSPHRASE);
    account.until("refused", |s, _| s.failures_total() > failures);
    let refusal = account.store.failures().last().unwrap();
    assert_eq!(refusal.method, methods::RESTORE_VERIFY);
    assert_eq!(refusal.message, "backend.backup.unreadable");
    world.stop();
}

#[test]
fn a_restore_is_cancelled_before_it_installs_and_jobs_come_back_after_a_reload() {
    let world = World::start("cancel", false);
    world.backup.archive("/b/good", archive(None, 0));
    let mut account = live(&world, false);
    verify(&mut account, "/b/good", PASSPHRASE);
    account.until("verifying", phase_is(BackupPhase::Verifying));
    // Without the stream, polls follow the running job.
    world.backup.step();
    account.until("progress by polling", |s, _| {
        job(s).is_some_and(|j| j.progress.items_done > 0 || j.progress.bytes_done > 0)
    });
    let id = job(&account.store).unwrap().id.clone();
    dispatch(&mut account, IntentKind::CancelBackupJob { job_id: id.clone() });
    account.until("cancelled", phase_is(BackupPhase::Cancelled));
    assert_eq!(world.requests("account.backup.cancel")[0].body["job_id"], id.as_str());

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
    let refused = handle_intent(
        &account.store,
        Intent::human(IntentKind::CreateBackup {
            account_id: AccountId::new(ACCOUNT),
            path: "/b/x".into(),
            secret: SecretInput::new(PASSPHRASE),
        }),
        Timestamp(0),
    );
    assert!(matches!(refused, Err(IntentError::UnsupportedByBackend { .. })));
    assert!(world.requests("account.backup.create").is_empty());
    world.stop();
}
