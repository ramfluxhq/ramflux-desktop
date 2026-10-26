// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Backup in the production window, on a fake daemon with the
//! BACKUP-01 job API as merged: a full backup needs a location and a
//! passphrase typed twice (with a strength hint), hands the passphrase to
//! the daemon as a secret, shows its progress with Cancel, then its counts
//! and size; a backup that fails on files it could not fetch offers, only
//! explicitly, to record them as missing; a restore checks the whole file
//! first, shows what it holds, its file check and that it merges (newer
//! wins), installs only after Restore, then shows what it installed, the
//! file check and what was kept because it was newer here, per category; a
//! file that does not match fails the restore before anything installs; a
//! wrong passphrase, a refused request and a daemon without the job API are
//! each explained.

mod common;
mod sdk_fake;

use std::sync::Arc;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use ramflux_desktop_core::{
    BackupFailure, BackupJob, BackupPhase, LockState, SecureSessions, SyncMode, selectors,
};
use ramflux_desktop_test::fake_backup::{Archive, Counts, FakeBackup};
use ramflux_desktop_test::fake_bus::FakeBus;
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::i18n::tr_args;
use ramflux_desktop_ui::route::{EntryStep, Page};
use ramflux_desktop_ui::settings::SettingsSection;
use ramflux_desktop_ui::settings::backup::{BackupPanel, Form};
use sdk_fake::wait;

const ACCOUNT: &str = "acct-main";
const PASSPHRASE: &str = "river stone lantern harbor";

type Handle = gpui_kit::WindowHandle<gpui_kit::base::Root>;

struct World {
    fake: Arc<FakeBus>,
    backup: Arc<FakeBackup>,
    handle: Handle,
    shell: gpui_kit::Entity<Shell>,
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

fn archive(mismatched: u64) -> Archive {
    Archive { mismatched, ..Archive::new(PASSPHRASE, ACCOUNT, "Personal", holdings()) }
}

fn open_backup(cx: &mut TestAppContext, name: &str, available: bool) -> World {
    let (fake, state, path) = sdk_fake::start(name);
    fake.enable_stream();
    let fake = Arc::new(fake);
    let backup = state.lock().expect("state").backup.clone();
    backup.set_available(available);
    backup.account(ACCOUNT, holdings());
    let bus = Arc::downgrade(&fake);
    backup.set_publisher(Box::new(move |row| {
        if let Some(bus) = bus.upgrade() {
            bus.publish_job(row);
        }
    }));
    let (handle, shell) = open_sized(cx, sdk_fake::options(&path), 900.);
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "the stream", |cx| {
        with_store(cx, &shell, |s| s.sync_status().is_some_and(|x| x.mode == SyncMode::Stream))
    });
    if available {
        wait(cx, "the job API", |cx| {
            with_store(cx, &shell, |s| {
                s.unsupported_reason(ramflux_desktop_core::methods::BACKUP_CREATE).is_none()
            })
        });
    }
    cx.update(|cx| {
        shell.update(cx, |shell, cx| shell.open_settings_at(SettingsSection::Backup, cx))
    });
    cx.run_until_parked();
    in_window(cx, handle, |_, _| {});
    World { fake, backup, handle, shell }
}

fn job(cx: &mut TestAppContext, world: &World) -> Option<BackupJob> {
    with_store(cx, &world.shell, |s| selectors::latest_backup_job(s).cloned())
}

fn phase(cx: &mut TestAppContext, world: &World) -> Option<BackupPhase> {
    job(cx, world).map(|j| j.phase)
}

fn disabled(cx: &mut TestAppContext, world: &World, id: &'static str) -> Option<bool> {
    in_window(cx, world.handle, move |window, _| window.find(id).disabled())
}

fn panel(
    cx: &mut TestAppContext,
    world: &World,
) -> gpui_kit::Entity<ramflux_desktop_ui::settings::backup::BackupPanel> {
    cx.update(|cx| world.shell.read(cx).settings().read(cx).backup_panel().clone())
}

fn form(cx: &mut TestAppContext, world: &World) -> Option<Form> {
    let panel = panel(cx, world);
    cx.update(|cx| panel.read(cx).form())
}

/// Sets one of the panel's masked inputs (0 new, 1 repeat, 2 restore).
fn set_input(cx: &mut TestAppContext, world: &World, index: usize, value: &'static str) {
    let panel = panel(cx, world);
    in_window(cx, world.handle, move |window, cx| {
        let input = panel.read(cx).inputs()[index].clone();
        input.update(cx, |input, cx| input.set_value(value, window, cx));
    });
}

fn input_empty(cx: &mut TestAppContext, world: &World, index: usize) -> bool {
    let panel = panel(cx, world);
    cx.update(|cx| panel.read(cx).inputs()[index].read(cx).value().is_empty())
}

fn requests(world: &World, method: &str) -> Vec<serde_json::Value> {
    world.fake.seen().into_iter().filter(|s| s.method == method).map(|s| s.body).collect()
}

fn choose_file(cx: &mut TestAppContext, world: &World, path: &'static str) {
    click(cx, world.handle, "restore-choose-file");
    assert!(cx.did_prompt_for_paths(), "the native open dialog opened");
    cx.simulate_path_prompt_response(move |options| {
        assert!(options.files && !options.directories && !options.multiple);
        Some(vec![path.into()])
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_full_backup_asks_where_and_a_passphrase_twice_then_shows_progress_and_counts(
    cx: &mut TestAppContext,
) {
    let world = open_backup(cx, "ui-backup-create", true);
    click(cx, world.handle, "backup-open");
    assert_eq!(form(cx, &world), Some(Form::Backup));
    assert_eq!(disabled(cx, &world, "backup-start"), Some(true), "nothing chosen yet");

    click(cx, world.handle, "backup-choose-location");
    cx.simulate_new_path_selection(|dir| Some(dir.join("personal.rfbackup")));
    cx.run_until_parked();
    set_input(cx, &world, 0, "short");
    assert!(exists(cx, world.handle, "backup-strength"), "a strength hint");
    assert_eq!(disabled(cx, &world, "backup-start"), Some(true), "too short");
    set_input(cx, &world, 0, PASSPHRASE);
    set_input(cx, &world, 1, "river stone");
    assert!(exists(cx, world.handle, "backup-mismatch"));
    assert_eq!(disabled(cx, &world, "backup-start"), Some(true), "not the same twice");
    set_input(cx, &world, 1, PASSPHRASE);
    assert!(!exists(cx, world.handle, "backup-mismatch"));
    assert_ne!(disabled(cx, &world, "backup-start"), Some(true));

    click(cx, world.handle, "backup-start");
    assert!(input_empty(cx, &world, 0) && input_empty(cx, &world, 1), "the secret left the inputs");
    wait(cx, "the job", |cx| phase(cx, &world).is_some());
    let sent = requests(&world, "account.backup.create");
    assert_eq!(sent[0]["passphrase"], PASSPHRASE);
    assert_eq!(sent[0]["allow_missing_attachments"], false, "every file, unless allowed");
    assert!(sent[0]["output_path"].as_str().is_some_and(|p| p.ends_with("/personal.rfbackup")));
    assert!(exists(cx, world.handle, "backup-job"));
    assert!(exists(cx, world.handle, "backup-progress"));
    assert!(exists(cx, world.handle, "backup-job-cancel"));
    assert!(!exists(cx, world.handle, "backup-open"), "one job at a time");

    for _ in 0..4 {
        world.backup.step();
    }
    wait(cx, "writing", |cx| {
        job(cx, &world)
            .is_some_and(|j| j.phase == BackupPhase::Writing && j.progress.bytes_done > 0)
    });
    world.backup.run_out();
    wait(cx, "done", |cx| phase(cx, &world) == Some(BackupPhase::Done));
    advance(cx, 50);
    assert!(exists(cx, world.handle, "backup-result"));
    assert_eq!(job(cx, &world).unwrap().summary.unwrap().counts.messages, 12_000);
    click(cx, world.handle, "backup-job-close");
    assert!(exists(cx, world.handle, "backup-open"), "closed: the forms again");
}

#[gpui_kit::test]
fn files_that_cannot_be_fetched_are_recorded_as_missing_only_when_asked(cx: &mut TestAppContext) {
    let world = open_backup(cx, "ui-backup-missing", true);
    world.backup.missing_files(ACCOUNT, 4, 7);
    click(cx, world.handle, "backup-open");
    click(cx, world.handle, "backup-choose-location");
    cx.simulate_new_path_selection(|dir| Some(dir.join("personal.rfbackup")));
    cx.run_until_parked();
    set_input(cx, &world, 0, PASSPHRASE);
    set_input(cx, &world, 1, PASSPHRASE);
    assert!(!exists(cx, world.handle, "backup-allow-missing-note"), "strict by default");
    click(cx, world.handle, "backup-start");
    wait(cx, "the job", |cx| phase(cx, &world).is_some());
    world.backup.step();
    wait(cx, "fetching", |cx| {
        job(cx, &world)
            .is_some_and(|j| j.progress.is_fetching(j.phase) && j.progress.items_done > 0)
    });
    world.backup.run_out();
    wait(cx, "failed", |cx| phase(cx, &world) == Some(BackupPhase::Failed));
    advance(cx, 50);
    assert!(matches!(
        job(cx, &world).unwrap().failure,
        Some(BackupFailure::AttachmentsUnavailable { count: 7, .. })
    ));
    assert!(exists(cx, world.handle, "backup-failure"));
    assert_eq!(requests(&world, "account.backup.create").len(), 1, "nothing sent on its own");

    click(cx, world.handle, "backup-allow-missing");
    assert_eq!(form(cx, &world), Some(Form::Backup));
    let panel = panel(cx, &world);
    assert_eq!(cx.update(|cx| panel.read(cx).allowed_missing()), Some(7));
    assert!(exists(cx, world.handle, "backup-allow-missing-note"), "it says what it records");
    assert_eq!(disabled(cx, &world, "backup-start"), Some(true), "the passphrase again");
    set_input(cx, &world, 0, PASSPHRASE);
    set_input(cx, &world, 1, PASSPHRASE);
    click(cx, world.handle, "backup-start");
    wait(cx, "the second job", |_| requests(&world, "account.backup.create").len() == 2);
    let sent = requests(&world, "account.backup.create");
    assert_eq!(sent[1]["allow_missing_attachments"], true);
    assert_eq!(sent[1]["output_path"], sent[0]["output_path"], "the same file");
    world.backup.run_out();
    wait(cx, "done", |cx| phase(cx, &world) == Some(BackupPhase::Done));
    let path = sent[1]["output_path"].as_str().unwrap().to_owned();
    assert_eq!(world.backup.archive_at(&path).unwrap().unavailable_at_source, 7);
    // The next backup is strict again.
    advance(cx, 50);
    click(cx, world.handle, "backup-job-close");
    click(cx, world.handle, "backup-open");
    assert!(!exists(cx, world.handle, "backup-allow-missing-note"));
}

#[gpui_kit::test]
fn a_file_that_does_not_match_fails_the_check_and_nothing_can_be_restored(cx: &mut TestAppContext) {
    let world = open_backup(cx, "ui-backup-mismatch", true);
    world.backup.archive("/b/personal.rfbackup", archive(2));
    click(cx, world.handle, "restore-open");
    choose_file(cx, &world, "/b/personal.rfbackup");
    set_input(cx, &world, 2, PASSPHRASE);
    click(cx, world.handle, "restore-verify");
    wait(cx, "checking", |cx| phase(cx, &world) == Some(BackupPhase::Verifying));
    world.backup.run_out();
    wait(cx, "failed", |cx| phase(cx, &world) == Some(BackupPhase::Failed));
    advance(cx, 50);
    assert!(exists(cx, world.handle, "backup-failure"));
    assert!(!exists(cx, world.handle, "backup-review"), "nothing to review");
    assert!(!exists(cx, world.handle, "restore-install"));
    let Some(BackupFailure::AttachmentMismatch { checks: Some(checks) }) =
        job(cx, &world).unwrap().failure
    else {
        panic!("expected a file mismatch");
    };
    assert_eq!(checks.mismatched, 2);
    assert!(requests(&world, "account.restore.install").is_empty());
}

#[gpui_kit::test]
fn a_restore_is_checked_reviewed_then_merged_with_its_attachment_check(cx: &mut TestAppContext) {
    let world = open_backup(cx, "ui-backup-restore", true);
    let recorded = Archive { unavailable_at_source: 3, ..archive(0) };
    world.backup.archive("/b/personal.rfbackup", recorded);
    world.backup.local_pending(ACCOUNT, 5);
    click(cx, world.handle, "restore-open");
    assert_eq!(disabled(cx, &world, "restore-verify"), Some(true));
    choose_file(cx, &world, "/b/personal.rfbackup");
    set_input(cx, &world, 2, PASSPHRASE);
    click(cx, world.handle, "restore-verify");
    assert!(input_empty(cx, &world, 2));
    wait(cx, "checking", |cx| phase(cx, &world) == Some(BackupPhase::Verifying));
    assert_eq!(requests(&world, "account.restore.verify")[0]["input_path"], "/b/personal.rfbackup");
    world.backup.run_out();
    wait(cx, "checked", |cx| phase(cx, &world) == Some(BackupPhase::Verified));
    advance(cx, 50);
    assert!(exists(cx, world.handle, "backup-review"));
    assert!(exists(cx, world.handle, "backup-review-effect"), "it says it merges, newer wins");
    assert!(exists(cx, world.handle, "backup-review-unavailable"), "recorded omissions named");
    assert!(requests(&world, "account.restore.install").is_empty(), "nothing before Restore");

    click(cx, world.handle, "restore-install");
    wait(cx, "installing", |cx| phase(cx, &world) == Some(BackupPhase::Installing));
    advance(cx, 50);
    assert!(!exists(cx, world.handle, "backup-job-cancel"), "installing cannot be cancelled");
    world.backup.run_out();
    wait(cx, "restored", |cx| phase(cx, &world) == Some(BackupPhase::Done));
    advance(cx, 50);
    assert!(exists(cx, world.handle, "backup-result"));
    assert!(exists(cx, world.handle, "restore-files"));
    assert!(exists(cx, world.handle, "restore-unavailable"));
    assert!(exists(cx, world.handle, "restore-kept-newer"), "what was newer here, per kind");
    let report = job(cx, &world).unwrap().report.unwrap();
    assert_eq!((report.attachments.verified, report.attachments.mismatched), (320, 0));
    assert!(report.attachments.all_verified());
    assert_eq!(report.attachments.local_missing_kept, 5);
    assert_eq!((report.kept_newer.messages, report.kept_newer.contacts), (2, 1));
}

#[gpui_kit::test]
fn a_wrong_passphrase_is_explained_and_the_file_is_kept_to_try_again(cx: &mut TestAppContext) {
    let world = open_backup(cx, "ui-backup-wrong", true);
    world.backup.archive("/b/personal.rfbackup", archive(0));
    click(cx, world.handle, "restore-open");
    choose_file(cx, &world, "/b/personal.rfbackup");
    set_input(cx, &world, 2, "wrong words entirely");
    click(cx, world.handle, "restore-verify");
    wait(cx, "checking", |cx| phase(cx, &world) == Some(BackupPhase::Verifying));
    world.backup.step();
    wait(cx, "failed", |cx| phase(cx, &world) == Some(BackupPhase::Failed));
    advance(cx, 50);
    assert_eq!(job(cx, &world).unwrap().failure, Some(BackupFailure::WrongPassphrase));
    assert!(exists(cx, world.handle, "backup-failure"));
    click(cx, world.handle, "backup-job-retry");
    assert_eq!(form(cx, &world), Some(Form::Restore));
    assert_eq!(disabled(cx, &world, "restore-verify"), Some(true), "the passphrase again");
    set_input(cx, &world, 2, PASSPHRASE);
    assert_ne!(disabled(cx, &world, "restore-verify"), Some(true), "the file is kept");

    // A request the daemon refuses is explained in the form: another
    // frontend's job runs (one at a time), not yet seen here.
    world.backup.set_publisher(Box::new(|_| {}));
    world.backup.archive("/b/other.rfbackup", archive(0));
    let other = serde_json::json!({ "input_path": "/b/other.rfbackup", "passphrase": PASSPHRASE });
    world.backup.handle("account.restore.verify", None, &other).expect("started");
    click(cx, world.handle, "restore-verify");
    wait(cx, "refused", |cx| exists(cx, world.handle, "backup-refused"));
}

#[gpui_kit::test]
fn without_the_job_api_backup_and_restore_are_disabled_with_the_reason(cx: &mut TestAppContext) {
    let world = open_backup(cx, "ui-backup-absent", false);
    advance(cx, 300);
    assert_eq!(disabled(cx, &world, "backup-open"), Some(true));
    assert_eq!(disabled(cx, &world, "restore-open"), Some(true));
    assert!(!requests(&world, "account.backup.jobs").is_empty(), "it asked");
}

/// The entry page's full-restore panel.
fn entry_panel(
    cx: &mut TestAppContext,
    shell: &gpui_kit::Entity<Shell>,
) -> gpui_kit::Entity<BackupPanel> {
    cx.update(|cx| shell.read(cx).entry().read(cx).full_restore().clone())
}

#[gpui_kit::test]
fn a_file_already_at_the_chosen_location_is_refused_in_the_form(cx: &mut TestAppContext) {
    let world = open_backup(cx, "ui-backup-exists", true);
    click(cx, world.handle, "backup-open");
    click(cx, world.handle, "backup-choose-location");
    cx.simulate_new_path_selection(|dir| {
        let taken = dir.join("taken.rfbackup");
        std::fs::write(&taken, b"an older backup").expect("a file there already");
        Some(taken)
    });
    cx.run_until_parked();
    set_input(cx, &world, 0, PASSPHRASE);
    set_input(cx, &world, 1, PASSPHRASE);
    assert!(exists(cx, world.handle, "backup-location-exists"), "the daemon never replaces a file");
    assert_eq!(disabled(cx, &world, "backup-start"), Some(true));
    click(cx, world.handle, "backup-choose-location");
    cx.simulate_new_path_selection(|dir| Some(dir.join("fresh.rfbackup")));
    cx.run_until_parked();
    assert!(!exists(cx, world.handle, "backup-location-exists"));
    assert_ne!(disabled(cx, &world, "backup-start"), Some(true));
}

#[gpui_kit::test]
fn secure_sessions_being_reestablished_show_a_banner_until_the_daemon_says_ready(
    cx: &mut TestAppContext,
) {
    let (fake, state, path) = sdk_fake::start("ui-backup-sessions");
    fake.enable_stream();
    state.lock().expect("state").secure_sessions = Some("reestablishing".to_owned());
    let (handle, shell) = open_sized(cx, sdk_fake::options(&path), 900.);
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "the stream", |cx| {
        with_store(cx, &shell, |s| s.sync_status().is_some_and(|x| x.mode == SyncMode::Stream))
    });
    let sessions = |cx: &mut TestAppContext| {
        with_store(cx, &shell, |s| s.accounts().next().map(|a| a.secure_sessions))
    };
    assert_eq!(sessions(cx), Some(SecureSessions::Reestablishing));
    advance(cx, 50);
    assert!(exists(cx, handle, "secure-sessions-banner"), "the workspace says so, persistently");
    fake.publish(
        ACCOUNT,
        "account.secure_sessions.changed",
        serde_json::json!({ "local_account_id": ACCOUNT, "secure_sessions": "ready" }),
    );
    wait(cx, "ready", |cx| sessions(cx) == Some(SecureSessions::Ready));
    advance(cx, 50);
    assert!(!exists(cx, handle, "secure-sessions-banner"));
    fake.shutdown();
}

#[gpui_kit::test]
fn a_fresh_device_restores_a_full_backup_from_the_entry_page(cx: &mut TestAppContext) {
    let (fake, state, path) = sdk_fake::start("ui-backup-entry");
    fake.enable_stream();
    let fake = Arc::new(fake);
    let backup = state.lock().expect("state").backup.clone();
    backup.archive("/b/personal.rfbackup", archive(0));
    let bus = Arc::downgrade(&fake);
    backup.set_publisher(Box::new(move |row| {
        if let Some(bus) = bus.upgrade() {
            bus.publish_job(row);
        }
    }));
    // No account on this device yet.
    state.lock().expect("state").accounts =
        Some(serde_json::json!({ "accounts": [], "account_details": [] }));
    let (handle, shell) = open_sized(cx, sdk_fake::options(&path), 900.);
    wait(cx, "the entry page", |cx| matches!(page(cx, &shell), Page::Entry(_)));
    if page(cx, &shell) == Page::Entry(EntryStep::Node) {
        click_in(cx, handle, "node-n-harbor", "node-confirm");
    }
    wait(cx, "the account step", |cx| page(cx, &shell) == Page::Entry(EntryStep::Account));
    wait(cx, "the job API at daemon scope", |cx| {
        with_store(cx, &shell, |s| {
            s.unsupported_reason(ramflux_desktop_core::methods::RESTORE_VERIFY).is_none()
        })
    });
    click(cx, handle, "entry-path-2");
    assert!(exists(cx, handle, "entry-full-restore"), "a full backup is the default");
    let panel = entry_panel(cx, &shell);
    click(cx, handle, "restore-choose-file");
    cx.simulate_path_prompt_response(|_| Some(vec!["/b/personal.rfbackup".into()]));
    cx.run_until_parked();
    in_window(cx, handle, move |window, cx| {
        let input = panel.read(cx).inputs()[2].clone();
        input.update(cx, |input, cx| input.set_value(PASSPHRASE, window, cx));
    });
    click(cx, handle, "restore-verify");
    wait(cx, "checking", |cx| {
        with_store(cx, &shell, |s| selectors::latest_backup_job(s).map(|j| j.phase))
            == Some(BackupPhase::Verifying)
    });
    let sent = fake.seen().into_iter().rfind(|s| s.method == "account.restore.verify");
    let sent = sent.expect("sent");
    assert_eq!(sent.account, None, "a daemon-level job");
    assert_eq!(sent.body["input_path"], "/b/personal.rfbackup");
    backup.run_out();
    wait(cx, "verified", |cx| {
        with_store(cx, &shell, |s| selectors::latest_backup_job(s).map(|j| j.phase))
            == Some(BackupPhase::Verified)
    });
    advance(cx, 50);
    assert!(exists(cx, handle, "backup-review"));
    // The daemon will list the account once it is installed.
    state.lock().expect("state").accounts = Some(serde_json::json!({
        "accounts": [],
        "account_details": [{ "id": ACCOUNT, "label": "Personal", "kind": "human",
                              "state": "locked" }],
    }));
    click(cx, handle, "restore-install");
    backup.run_out();
    wait(cx, "restored", |cx| {
        with_store(cx, &shell, |s| selectors::latest_backup_job(s).map(|j| j.phase))
            == Some(BackupPhase::Done)
    });
    wait(cx, "the account listed", |cx| {
        with_store(cx, &shell, |s| s.accounts().next().map(|a| a.lock_state))
            == Some(LockState::Locked)
    });
    advance(cx, 50);
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Account));
    assert!(exists(cx, handle, "backup-result"), "the result stays in view");
    assert!(exists(cx, handle, "passphrase"), "and the account can be unlocked");
    if let Ok(fake) = Arc::try_unwrap(fake) {
        fake.shutdown();
    }
}

/// A fresh device whose full restore is checked and awaits Restore on the
/// entry page, the daemon reopening the account once it installs. The
/// reopened account's first jobs listing is held back (`account_jobs`), so
/// the interval before it is observed, not raced.
struct Restoring {
    fake: Arc<FakeBus>,
    state: sdk_fake::Shared,
    backup: Arc<FakeBackup>,
    handle: Handle,
    shell: gpui_kit::Entity<Shell>,
}

fn restoring(cx: &mut TestAppContext, name: &str) -> Restoring {
    let (fake, state, path) = sdk_fake::start(name);
    fake.enable_stream();
    let fake = Arc::new(fake);
    let backup = state.lock().expect("state").backup.clone();
    backup.archive("/b/personal.rfbackup", archive(0));
    let bus = Arc::downgrade(&fake);
    backup.set_publisher(Box::new(move |row| {
        if let Some(bus) = bus.upgrade() {
            bus.publish_job(row);
        }
    }));
    state.lock().expect("state").accounts =
        Some(serde_json::json!({ "accounts": [], "account_details": [] }));
    // The held jobs listing must outlive the usual 3 s deadline: the test
    // opens it on cue, never on a timer.
    let mut options = sdk_fake::options(&path);
    if let ramflux_desktop_ui::shell::SourceChoice::Rfd(config) = &mut options.source {
        config.deadlines =
            ramflux_desktop_sdk::Deadlines::uniform(std::time::Duration::from_secs(120));
    }
    let (handle, shell) = open_sized(cx, options, 900.);
    wait(cx, "the entry page", |cx| matches!(page(cx, &shell), Page::Entry(_)));
    if page(cx, &shell) == Page::Entry(EntryStep::Node) {
        click_in(cx, handle, "node-n-harbor", "node-confirm");
    }
    wait(cx, "the account step", |cx| page(cx, &shell) == Page::Entry(EntryStep::Account));
    wait(cx, "the job API at daemon scope", |cx| {
        with_store(cx, &shell, |s| {
            s.unsupported_reason(ramflux_desktop_core::methods::RESTORE_VERIFY).is_none()
        })
    });
    click(cx, handle, "entry-path-2");
    let panel = entry_panel(cx, &shell);
    click(cx, handle, "restore-choose-file");
    cx.simulate_path_prompt_response(|_| Some(vec!["/b/personal.rfbackup".into()]));
    cx.run_until_parked();
    in_window(cx, handle, move |window, cx| {
        let input = panel.read(cx).inputs()[2].clone();
        input.update(cx, |input, cx| input.set_value(PASSPHRASE, window, cx));
    });
    click(cx, handle, "restore-verify");
    backup.run_out();
    wait(cx, "verified", |cx| {
        with_store(cx, &shell, |s| selectors::latest_backup_job(s).map(|j| j.phase))
            == Some(BackupPhase::Verified)
    });
    advance(cx, 50);
    Restoring { fake, state, backup, handle, shell }
}

impl Restoring {
    fn set_account(&self, state: &str) {
        let open = state == "unlocked";
        self.state.lock().expect("state").accounts = Some(serde_json::json!({
            "accounts": if open { vec![ACCOUNT] } else { vec![] },
            "account_details": [{ "id": ACCOUNT, "label": "Personal", "kind": "human",
                                  "state": state }],
        }));
    }

    /// Installs; as the real daemon does, the account is reopened,
    /// unlocked, and its session's first jobs listing is held back.
    fn install_and_hold(&self, cx: &mut TestAppContext) -> Arc<sdk_fake::Latch> {
        self.set_account("unlocked");
        let account_jobs = self.state.lock().expect("state").account_jobs.clone();
        account_jobs.close();
        click(cx, self.handle, "restore-install");
        self.backup.run_out();
        wait(cx, "the account's own session", |cx| {
            let store = store_handle(cx, &self.shell);
            cx.update(|cx| store.read(cx).active_account().is_some())
        });
        advance(cx, 300);
        account_jobs
    }

    fn step(&self, cx: &mut TestAppContext) -> EntryStep {
        cx.update(|cx| self.shell.read(cx).entry().read(cx).step(cx))
    }

    fn finish(self) {
        if let Ok(fake) = Arc::try_unwrap(self.fake) {
            fake.shutdown();
        }
    }
}

#[gpui_kit::test]
fn the_restore_result_stays_in_view_when_the_daemon_reopens_the_account(cx: &mut TestAppContext) {
    let world = restoring(cx, "ui-backup-entry-open");
    let account_jobs = world.install_and_hold(cx);
    assert!(
        with_store(cx, &world.shell, |s| !s.is_listed(ramflux_desktop_core::Listing::BackupJobs)),
        "the account session has listed nothing yet"
    );
    // What is rendered, not only what is routed: the account step with the
    // restore result as last seen, never the daemon step.
    assert_eq!(page(cx, &world.shell), Page::Entry(EntryStep::Account));
    assert_eq!(world.step(cx), EntryStep::Account, "the rendered step follows the hold");
    assert!(exists(cx, world.handle, "entry-restore-finishing"));
    // The title bar reads the daemon's own status while the account session
    // is silent: the daemon is up, so it does not say it is disconnected.
    let connected =
        ramflux_desktop_ui::i18n::source_status(ramflux_desktop_core::SourceStatus::Connected);
    assert_eq!(source_shown(cx, &world).as_deref(), Some(connected.as_ref()));
    assert!(exists(cx, world.handle, "backup-result"), "the last seen result stays in view");
    assert!(!exists(cx, world.handle, "passphrase"), "no unlock form for an open account");
    account_jobs.open();
    wait(cx, "the jobs listed", |cx| {
        with_store(cx, &world.shell, |s| {
            s.is_listed(ramflux_desktop_core::Listing::BackupJobs)
                && s.accounts().next().map(|a| a.lock_state) == Some(LockState::Unlocked)
        })
    });
    advance(cx, 300);
    assert_eq!(page(cx, &world.shell), Page::Entry(EntryStep::Account), "the result is held");
    assert!(!exists(cx, world.handle, "entry-restore-finishing"));
    assert!(exists(cx, world.handle, "backup-result"), "counts, kept-newer and the file check");
    assert!(exists(cx, world.handle, "entry-open-account"));
    click(cx, world.handle, "entry-open-account");
    wait(cx, "the workspace", |cx| page(cx, &world.shell) == Page::Workspace);
    world.finish();
}

#[gpui_kit::test]
fn a_daemon_that_goes_away_during_the_hold_releases_it(cx: &mut TestAppContext) {
    let world = restoring(cx, "ui-backup-entry-gone");
    let account_jobs = world.install_and_hold(cx);
    assert_eq!(world.step(cx), EntryStep::Account, "held while the daemon is there");
    // The daemon stops: the page falls back to the daemon step, which has
    // the daemon controls and says it is not reachable.
    let Restoring { fake, handle, shell, .. } = world;
    let fake = Arc::try_unwrap(fake).ok().expect("only the test holds the daemon");
    fake.shutdown();
    wait(cx, "the daemon is seen gone", |cx| {
        let store = store_handle(cx, &shell);
        cx.update(|cx| store.read(cx).daemon_unreachable())
    });
    wait(cx, "the daemon step", |cx| {
        cx.update(|cx| shell.read(cx).entry().read(cx).step(cx)) == EntryStep::Daemon
    });
    assert!(!cx.update(|cx| shell.read(cx).entry().read(cx).awaits_restore_listing(cx)));
    assert!(exists(cx, handle, "entry-daemon-socket"), "the daemon step is rendered");
    // Only now let the held listing go (its answer has nowhere to arrive).
    account_jobs.open();
}

#[gpui_kit::test]
fn only_the_installed_restore_is_held_not_an_old_one_after_lock_and_unlock(
    cx: &mut TestAppContext,
) {
    let world = restoring(cx, "ui-backup-entry-relock");
    let account_jobs = world.install_and_hold(cx);
    account_jobs.open();
    wait(cx, "the jobs listed", |cx| {
        with_store(cx, &world.shell, |s| s.is_listed(ramflux_desktop_core::Listing::BackupJobs))
    });
    advance(cx, 300);
    let awaits = |cx: &mut TestAppContext| {
        cx.update(|cx| world.shell.read(cx).entry().read(cx).awaits_restore_listing(cx))
    };
    assert!(!awaits(cx), "the first listing ended the hold");
    // The account locks and opens again: a fresh session starts empty, and
    // the old, unclosed result must not take the page over again.
    world.set_account("locked");
    wait(cx, "locked", |cx| {
        with_store(cx, &world.shell, |s| {
            s.accounts().next().map(|a| a.lock_state) != Some(LockState::Unlocked)
        })
    });
    account_jobs.close();
    world.set_account("unlocked");
    wait(cx, "the new session", |cx| {
        with_store(cx, &world.shell, |s| !s.is_listed(ramflux_desktop_core::Listing::BackupJobs))
    });
    advance(cx, 300);
    assert!(!awaits(cx), "no hold: the installed restore is not awaited any more");
    account_jobs.open();
    world.finish();
}

/// The source status the title bar shows.
fn source_shown(cx: &mut TestAppContext, world: &Restoring) -> Option<String> {
    in_window(cx, world.handle, |window, _| {
        window.try_find("status-source-value").and_then(|e| e.label().map(str::to_owned))
    })
}

#[gpui_kit::test]
fn a_freshly_unlocked_account_shows_a_loading_line_not_the_daemon_step(cx: &mut TestAppContext) {
    let world = restoring(cx, "ui-backup-entry-fresh");
    let account_jobs = world.install_and_hold(cx);
    account_jobs.open();
    wait(cx, "the jobs listed", |cx| {
        with_store(cx, &world.shell, |s| s.is_listed(ramflux_desktop_core::Listing::BackupJobs))
    });
    advance(cx, 300);
    // The account locks and opens again, and its new session has received
    // nothing yet.
    world.set_account("locked");
    wait(cx, "locked", |cx| {
        with_store(cx, &world.shell, |s| {
            s.accounts().next().map(|a| a.lock_state) != Some(LockState::Unlocked)
        })
    });
    let account_reads = world.state.lock().expect("state").account_reads.clone();
    account_reads.close();
    world.set_account("unlocked");
    wait(cx, "the new session", |cx| {
        with_store(cx, &world.shell, |s| !s.is_listed(ramflux_desktop_core::Listing::BackupJobs))
    });
    advance(cx, 300);
    // What is rendered: the account step with a neutral line, never the
    // daemon step (the daemon is there).
    assert_eq!(world.step(cx), EntryStep::Account);
    assert!(exists(cx, world.handle, "entry-session-loading"));
    assert!(!exists(cx, world.handle, "entry-daemon-socket"), "no daemon step is rendered");
    // The page value agrees with the rendered step, and the title bar
    // reads the daemon's own status, not the empty session's.
    assert_eq!(page(cx, &world.shell), Page::Entry(EntryStep::Account));
    let connected =
        ramflux_desktop_ui::i18n::source_status(ramflux_desktop_core::SourceStatus::Connected);
    assert_eq!(source_shown(cx, &world).as_deref(), Some(connected.as_ref()));
    account_reads.open();
    wait(cx, "the first data", |cx| !exists(cx, world.handle, "entry-session-loading"));
    world.finish();
}

/// A restored account that is locked and opened again while its new
/// session's answers are held back for good: the loading line first, then
/// (past the bound) the error. Returns the latch holding the answers.
fn never_loading(cx: &mut TestAppContext, world: &Restoring) -> Arc<sdk_fake::Latch> {
    let account_jobs = world.install_and_hold(cx);
    account_jobs.open();
    wait(cx, "the jobs listed", |cx| {
        with_store(cx, &world.shell, |s| s.is_listed(ramflux_desktop_core::Listing::BackupJobs))
    });
    advance(cx, 300);
    world.set_account("locked");
    wait(cx, "locked", |cx| {
        with_store(cx, &world.shell, |s| {
            s.accounts().next().map(|a| a.lock_state) != Some(LockState::Unlocked)
        })
    });
    let account_reads = world.state.lock().expect("state").account_reads.clone();
    account_reads.close();
    world.set_account("unlocked");
    wait(cx, "the loading line", |cx| exists(cx, world.handle, "entry-session-loading"));
    assert!(!exists(cx, world.handle, "entry-session-retry"), "nothing to retry yet");
    // Past the bound with no answer: the error and the ways out replace
    // the spinner.
    advance(cx, 10_500);
    assert!(exists(cx, world.handle, "entry-session-failed"));
    assert!(exists(cx, world.handle, "entry-session-retry"));
    assert!(exists(cx, world.handle, "entry-session-lock"));
    // Both name the account by its label.
    let named = |cx: &mut TestAppContext, id: &'static str| {
        in_window(cx, world.handle, move |window, _| {
            window.try_find(id).and_then(|e| e.label().map(str::to_owned))
        })
    };
    let failed = tr_args("entry.account.load_failed", &[("name", "Personal")]);
    let lock = tr_args("entry.account.lock", &[("name", "Personal")]);
    assert_eq!(
        named(cx, "entry-session-failed-text"),
        Some(failed.to_string()),
        "the error names it"
    );
    assert_eq!(named(cx, "entry-session-lock"), Some(lock.to_string()), "the button names it");
    assert!(!exists(cx, world.handle, "entry-session-loading"));
    assert_eq!(world.step(cx), EntryStep::Account);
    account_reads
}

fn status_reads(world: &Restoring) -> usize {
    world.fake.seen().iter().filter(|s| s.account.is_some() && s.method == "account.status").count()
}

#[gpui_kit::test]
fn an_account_that_never_loads_says_so_and_try_again_opens_it_anew(cx: &mut TestAppContext) {
    let world = restoring(cx, "ui-backup-entry-never");
    let account_reads = never_loading(cx, &world);
    let before = status_reads(&world);
    click(cx, world.handle, "entry-session-retry");
    // The loading line is back, and the daemon is asked again.
    wait(cx, "the loading line again", |cx| exists(cx, world.handle, "entry-session-loading"));
    assert!(!exists(cx, world.handle, "entry-session-retry"));
    wait(cx, "the daemon asked again", |_| status_reads(&world) > before);
    // This time the account answers.
    account_reads.open();
    wait(cx, "the account loaded", |cx| {
        !exists(cx, world.handle, "entry-session-loading")
            && !exists(cx, world.handle, "entry-session-failed")
    });
    world.finish();
}

#[gpui_kit::test]
fn an_account_that_never_loads_can_be_locked_from_the_error(cx: &mut TestAppContext) {
    let world = restoring(cx, "ui-backup-entry-never-lock");
    let account_reads = never_loading(cx, &world);
    click(cx, world.handle, "entry-session-lock");
    wait(cx, "the daemon asked to lock it", |_| {
        world.fake.seen().iter().any(|s| s.method == "account.lock")
    });
    account_reads.open();
    world.finish();
}

#[gpui_kit::test]
fn a_daemon_without_the_job_api_offers_the_key_backup_first(cx: &mut TestAppContext) {
    let (fake, state, path) = sdk_fake::start("ui-backup-entry-key");
    fake.enable_stream();
    {
        let state = state.lock().expect("state");
        state.backup.set_available(false);
    }
    state.lock().expect("state").accounts =
        Some(serde_json::json!({ "accounts": [], "account_details": [] }));
    let (handle, shell) = open_sized(cx, sdk_fake::options(&path), 900.);
    wait(cx, "the entry page", |cx| matches!(page(cx, &shell), Page::Entry(_)));
    if page(cx, &shell) == Page::Entry(EntryStep::Node) {
        click_in(cx, handle, "node-n-harbor", "node-confirm");
    }
    wait(cx, "the account step", |cx| page(cx, &shell) == Page::Entry(EntryStep::Account));
    advance(cx, 300);
    click(cx, handle, "entry-path-2");
    assert!(!exists(cx, handle, "entry-full-restore"), "the key backup is the default");
    assert!(exists(cx, handle, "backup-browse"));
    // The full restore can still be chosen; its Check is gated.
    click(cx, handle, "entry-restore-kind-0");
    assert!(exists(cx, handle, "entry-full-restore"));
    let gated = in_window(cx, handle, |window, _| window.find("restore-verify").disabled());
    assert_eq!(gated, Some(true));
    fake.shutdown();
}
