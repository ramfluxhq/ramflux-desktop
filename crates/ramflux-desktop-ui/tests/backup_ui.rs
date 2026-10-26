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

#[gpui_kit::test]
fn the_restore_result_stays_in_view_when_the_daemon_reopens_the_account(cx: &mut TestAppContext) {
    let (fake, state, path) = sdk_fake::start("ui-backup-entry-open");
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
    // As the real daemon does: once installed, the account is reopened,
    // unlocked.
    state.lock().expect("state").accounts = Some(serde_json::json!({
        "accounts": [ACCOUNT],
        "account_details": [{ "id": ACCOUNT, "label": "Personal", "kind": "human",
                              "state": "unlocked" }],
    }));
    // The reopened account's session starts empty: its first jobs listing is
    // held back, so the interval before it is observed, not raced.
    let account_jobs = state.lock().expect("state").account_jobs.clone();
    account_jobs.close();
    click(cx, handle, "restore-install");
    backup.run_out();
    wait(cx, "the account's own session", |cx| {
        let store = store_handle(cx, &shell);
        cx.update(|cx| store.read(cx).active_account().is_some())
    });
    advance(cx, 300);
    assert!(
        with_store(cx, &shell, |s| !s.is_listed(ramflux_desktop_core::Listing::BackupJobs)),
        "the account session has listed nothing yet"
    );
    assert_eq!(
        page(cx, &shell),
        Page::Entry(EntryStep::Account),
        "the entry page is held until the account's first jobs listing: no flash of another page"
    );
    account_jobs.open();
    wait(cx, "the jobs listed", |cx| {
        with_store(cx, &shell, |s| {
            s.is_listed(ramflux_desktop_core::Listing::BackupJobs)
                && s.accounts().next().map(|a| a.lock_state) == Some(LockState::Unlocked)
        })
    });
    advance(cx, 300);
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Account), "the result is held in view");
    assert!(exists(cx, handle, "backup-result"), "counts, kept-newer and the file check");
    assert!(exists(cx, handle, "entry-open-account"));
    click(cx, handle, "entry-open-account");
    wait(cx, "the workspace", |cx| page(cx, &shell) == Page::Workspace);
    if let Ok(fake) = Arc::try_unwrap(fake) {
        fake.shutdown();
    }
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
