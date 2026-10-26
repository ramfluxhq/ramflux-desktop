// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Full backup and restore jobs in the store (the job state machine,
//! versions, identity) and in policy (human only, one job at a time,
//! install only a verified restore, cancel until it installs).

mod common;

use common::*;
use ramflux_desktop_core::*;

fn account() -> Account {
    serde_json::from_value(serde_json::json!({
        "id": "acct-main", "label": "Main", "lock_state": "Unlocked",
    }))
    .expect("account")
}

fn job(id: &str, kind: BackupJobKind, phase: BackupPhase, version: u64) -> BackupJob {
    BackupJob {
        id: BackupJobId::new(id),
        kind,
        path: "/b/personal.rfbackup".to_owned(),
        phase,
        version,
        started_at: Timestamp(1_000),
        finished_at: None,
        progress: BackupProgress::default(),
        summary: None,
        preview: None,
        report: None,
        failure: None,
    }
}

fn restore(phase: BackupPhase, version: u64) -> DomainEvent {
    DomainEvent::BackupJobUpdated(Box::new(job("j-1", BackupJobKind::Restore, phase, version)))
}

fn feed() -> Feed {
    let mut feed = Feed::new();
    feed.ok(1, DomainEvent::AccountUpdated(account()));
    feed
}

fn decide(store: &Store, kind: IntentKind) -> Result<Vec<Effect>, IntentError> {
    handle_intent(store, Intent::human(kind), Timestamp(10))
}

fn create(path: &str, secret: &str) -> IntentKind {
    IntentKind::CreateBackup {
        account_id: AccountId::new("acct-main"),
        path: path.into(),
        secret: SecretInput::new(secret),
        allow_missing_attachments: false,
    }
}

fn backup(phase: BackupPhase, version: u64) -> DomainEvent {
    DomainEvent::BackupJobUpdated(Box::new(job("b-1", BackupJobKind::Backup, phase, version)))
}

#[test]
fn a_restore_moves_through_verification_review_and_installation() {
    let mut feed = feed();
    let changes = feed.ok(2, restore(BackupPhase::Verifying, 1));
    assert!(changes.backup_jobs.contains(&BackupJobId::new("j-1")));
    feed.ok(3, restore(BackupPhase::Verifying, 2));
    feed.ok(4, restore(BackupPhase::Verified, 3));
    feed.ok(5, restore(BackupPhase::Installing, 4));
    feed.ok(6, restore(BackupPhase::Done, 5));
    assert_eq!(selectors::latest_backup_job(&feed.store).unwrap().phase, BackupPhase::Done);
    assert!(selectors::running_backup_job(&feed.store).is_none());
}

#[test]
fn job_updates_are_validated_in_full() {
    let mut feed = feed();
    feed.ok(2, restore(BackupPhase::Verifying, 3));
    assert!(matches!(
        feed.push(3, restore(BackupPhase::Verified, 3)),
        Err(ApplyError::StaleBackupVersion { current: 3, found: 3, .. })
    ));
    // Nothing installs before it verified.
    assert!(matches!(
        feed.push(4, restore(BackupPhase::Installing, 4)),
        Err(ApplyError::IllegalBackupTransition { .. })
    ));
    let mut moved = job("j-1", BackupJobKind::Restore, BackupPhase::Verified, 5);
    moved.path = "/elsewhere".to_owned();
    assert_eq!(
        feed.push(5, DomainEvent::BackupJobUpdated(Box::new(moved))),
        Err(ApplyError::BackupJobChanged(BackupJobId::new("j-1")))
    );
    feed.ok(6, restore(BackupPhase::Verified, 6));
    feed.ok(7, restore(BackupPhase::Installing, 7));
    // An installing restore cannot be cancelled.
    assert!(feed.push(8, restore(BackupPhase::Cancelled, 8)).is_err());
    // A backup never verifies; a restore never writes.
    let odd = job("j-2", BackupJobKind::Backup, BackupPhase::Verifying, 1);
    assert!(feed.push(9, DomainEvent::BackupJobUpdated(Box::new(odd))).is_err());
    let odd = job("j-3", BackupJobKind::Restore, BackupPhase::Writing, 1);
    assert!(feed.push(10, DomainEvent::BackupJobUpdated(Box::new(odd))).is_err());
    assert_eq!(feed.store.backup_job(&BackupJobId::new("j-1")).unwrap().version, 7);
}

#[test]
fn a_backup_writes_only_after_preparing_as_the_daemon_moves_it() {
    let mut feed = feed();
    feed.ok(2, backup(BackupPhase::Preparing, 1));
    // The daemon never skips writing (its machine: preparing -> writing).
    assert!(matches!(
        feed.push(3, backup(BackupPhase::Done, 2)),
        Err(ApplyError::IllegalBackupTransition { .. })
    ));
    feed.ok(4, backup(BackupPhase::Writing, 3));
    feed.ok(5, backup(BackupPhase::Done, 4));
    assert_eq!(feed.store.backup_job(&BackupJobId::new("b-1")).unwrap().phase, BackupPhase::Done);
}

#[test]
fn of_two_jobs_started_in_the_same_second_the_one_seen_last_is_the_latest() {
    let mut feed = feed();
    // A backup failed and was made again within the same second; the
    // retry's id sorts first.
    let mut failed = job("zz-first", BackupJobKind::Backup, BackupPhase::Failed, 3);
    failed.failure = Some(BackupFailure::AttachmentsUnavailable {
        count: 2,
        selectors: vec!["obj-1".to_owned(), "obj-2".to_owned()],
        next_cursor: None,
    });
    feed.ok(2, DomainEvent::BackupJobUpdated(Box::new(failed)));
    let retry = job("aa-retry", BackupJobKind::Backup, BackupPhase::Preparing, 1);
    feed.ok(3, DomainEvent::BackupJobUpdated(Box::new(retry)));
    let latest = selectors::latest_backup_job(&feed.store).unwrap();
    assert_eq!(latest.id, BackupJobId::new("aa-retry"));
    assert_eq!(
        selectors::running_backup_job(&feed.store).unwrap().id,
        BackupJobId::new("aa-retry")
    );
}

#[test]
fn a_file_check_passes_only_with_every_file_matched_or_recorded_missing() {
    let recorded = AttachmentCheck {
        verified: 10,
        missing: 2,
        unavailable_at_source: 2,
        local_missing_kept: 7,
        ..AttachmentCheck::default()
    };
    assert!(recorded.all_verified(), "recorded omissions and local pending files pass");
    assert_eq!(recorded.unapproved_missing(), 0);
    let unrecorded = AttachmentCheck { missing: 3, ..recorded };
    assert!(!unrecorded.all_verified());
    assert_eq!(unrecorded.unapproved_missing(), 1);
    let mismatched = AttachmentCheck { mismatched: 1, ..recorded };
    assert!(!mismatched.all_verified());
}

#[test]
fn fetching_is_a_preparing_backup_counting_files() {
    let fetching = BackupProgress {
        category: BackupCategory::Attachments,
        items_done: 2,
        items_total: Some(8),
        ..BackupProgress::default()
    };
    assert!(fetching.is_fetching(BackupPhase::Preparing));
    assert!(!fetching.is_fetching(BackupPhase::Writing));
    let snapshot = BackupProgress { category: BackupCategory::Identity, ..fetching };
    assert!(!snapshot.is_fetching(BackupPhase::Preparing));
    assert_eq!(fetching.fraction(), Some(0.25));
}

#[test]
fn progress_reads_bytes_first_then_items() {
    let bytes = BackupProgress {
        items_done: 1,
        items_total: Some(4),
        bytes_done: 30,
        bytes_total: Some(40),
        ..BackupProgress::default()
    };
    assert_eq!(bytes.fraction(), Some(0.75));
    let items = BackupProgress { items_done: 1, items_total: Some(4), ..BackupProgress::default() };
    assert_eq!(items.fraction(), Some(0.25));
    assert_eq!(BackupProgress::default().fraction(), None);
    let empty = BackupProgress { bytes_total: Some(0), ..BackupProgress::default() };
    assert_eq!(empty.fraction(), Some(1.0));
}

#[test]
fn starting_a_job_carries_the_passphrase_as_a_secret_one_job_at_a_time() {
    let mut feed = feed();
    let effects = decide(&feed.store, create("/b/new.rfbackup", "quiet harbor lights")).unwrap();
    let [Effect::SecretCall { method, body, secrets }] = effects.as_slice() else {
        panic!("a secret call");
    };
    assert_eq!(method, methods::BACKUP_CREATE);
    assert_eq!(body["path"], "/b/new.rfbackup");
    assert_eq!(body["account_id"], "acct-main");
    assert_eq!(body["allow_missing_attachments"], false, "strict by default");
    assert!(!body.to_string().contains("quiet harbor"));
    assert_eq!(secrets[0].expose(), "quiet harbor lights");
    let verify = IntentKind::VerifyRestore { path: "/b/x".into(), secret: SecretInput::new("pw") };
    let verified = decide(&feed.store, verify).unwrap();
    assert_eq!(verified[0].method(), Some(methods::RESTORE_VERIFY));
    assert!(matches!(&verified[0], Effect::SecretCall { secrets, .. } if secrets.len() == 1));
    assert_eq!(
        decide(&feed.store, create("/b/x", "")).unwrap_err(),
        IntentError::Empty("backup passphrase")
    );
    assert_eq!(
        decide(&feed.store, create(" ", "pw")).unwrap_err(),
        IntentError::Empty("backup path")
    );

    feed.ok(2, restore(BackupPhase::Verifying, 1));
    assert_eq!(
        decide(&feed.store, create("/b/y", "pw")).unwrap_err(),
        IntentError::BackupInProgress
    );
    let verify = IntentKind::VerifyRestore { path: "/b/x".into(), secret: SecretInput::new("pw") };
    assert_eq!(decide(&feed.store, verify).unwrap_err(), IntentError::BackupInProgress);
    feed.ok(3, restore(BackupPhase::Failed, 2));
    assert!(decide(&feed.store, create("/b/y", "pw")).is_ok());
    // Recording unavailable files is asked for explicitly.
    let allowed = IntentKind::CreateBackup {
        account_id: AccountId::new("acct-main"),
        path: "/b/y".into(),
        secret: SecretInput::new("pw"),
        allow_missing_attachments: true,
    };
    let effects = decide(&feed.store, allowed).unwrap();
    assert_eq!(effects[0].body().unwrap()["allow_missing_attachments"], true);
}

#[test]
fn only_a_verified_restore_installs_and_cancelling_ends_before_installing() {
    let mut feed = feed();
    let id = BackupJobId::new("j-1");
    let install = || IntentKind::InstallRestore { job_id: BackupJobId::new("j-1") };
    let cancel = || IntentKind::CancelBackupJob { job_id: BackupJobId::new("j-1") };
    assert_eq!(
        decide(&feed.store, install()).unwrap_err(),
        IntentError::UnknownBackupJob(id.clone())
    );
    feed.ok(2, restore(BackupPhase::Verifying, 1));
    assert_eq!(
        decide(&feed.store, install()).unwrap_err(),
        IntentError::BackupJobPhase(id.clone())
    );
    assert!(decide(&feed.store, cancel()).is_ok());
    feed.ok(3, restore(BackupPhase::Verified, 2));
    let effects = decide(&feed.store, install()).unwrap();
    assert_eq!(effects[0].method(), Some(methods::RESTORE_INSTALL));
    assert_eq!(effects[0].body().unwrap()["job_id"], "j-1");
    assert_eq!(decide(&feed.store, cancel()).unwrap()[0].method(), Some(methods::BACKUP_CANCEL));
    feed.ok(4, restore(BackupPhase::Installing, 3));
    assert_eq!(decide(&feed.store, cancel()).unwrap_err(), IntentError::BackupJobPhase(id.clone()));
    // A backup is never installed.
    feed.ok(
        5,
        DomainEvent::BackupJobUpdated(Box::new(job(
            "j-2",
            BackupJobKind::Backup,
            BackupPhase::Done,
            1,
        ))),
    );
    let install_backup = IntentKind::InstallRestore { job_id: BackupJobId::new("j-2") };
    assert!(matches!(decide(&feed.store, install_backup), Err(IntentError::BackupJobPhase(_))));
}

#[test]
fn agents_never_back_up_or_restore() {
    let mut feed = feed();
    feed.ok(
        1,
        DomainEvent::ParticipantUpserted(participant(
            "p-bot",
            ParticipantKind::Agent,
            "Bot",
            false,
        )),
    );
    feed.ok(2, restore(BackupPhase::Verified, 1));
    let agent = |kind| {
        handle_intent(&feed.store, Intent::agent(ParticipantId::new("p-bot"), kind), Timestamp(10))
    };
    for kind in [
        create("/b/x", "pw"),
        IntentKind::VerifyRestore { path: "/b/x".into(), secret: SecretInput::new("pw") },
        IntentKind::InstallRestore { job_id: BackupJobId::new("j-1") },
        IntentKind::CancelBackupJob { job_id: BackupJobId::new("j-1") },
    ] {
        assert!(matches!(agent(kind), Err(IntentError::AgentForbidden(_))));
    }
}

#[test]
fn a_backend_without_the_job_api_refuses_every_backup_intent() {
    let mut feed = feed();
    let unsupported = [
        methods::BACKUP_CREATE,
        methods::RESTORE_VERIFY,
        methods::RESTORE_INSTALL,
        methods::BACKUP_CANCEL,
    ]
    .into_iter()
    .map(|m| (m.to_owned(), "backend.gap.full_backup".to_owned()))
    .collect();
    feed.ok(1, DomainEvent::CapabilitiesUpdated(Capabilities { unsupported }));
    assert!(matches!(
        decide(&feed.store, create("/b/x", "pw")),
        Err(IntentError::UnsupportedByBackend { method: "backup.create", .. })
    ));
}
