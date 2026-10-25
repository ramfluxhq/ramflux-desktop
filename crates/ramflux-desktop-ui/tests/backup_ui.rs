// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Backup in the production window, on a fake daemon with the
//! assumed BACKUP-01 job API: a full backup needs a location and a
//! passphrase typed twice (with a strength hint), hands the passphrase to
//! the daemon as a secret, shows its progress with Cancel, then its counts
//! and size; a restore checks the whole file first, shows what it holds and
//! that it merges (newer wins), installs only after Restore, then shows
//! what it installed and the attachment check; a wrong passphrase, a
//! refused file and a daemon without the job API are each explained.

mod common;
mod sdk_fake;

use std::sync::Arc;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use ramflux_desktop_core::{BackupFailure, BackupJob, BackupPhase, SyncMode, selectors};
use ramflux_desktop_test::fake_backup::{Archive, Counts, FakeBackup};
use ramflux_desktop_test::fake_bus::FakeBus;
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::route::Page;
use ramflux_desktop_ui::settings::SettingsSection;
use ramflux_desktop_ui::settings::backup::Form;
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
    Archive {
        passphrase: PASSPHRASE.to_owned(),
        account_id: ACCOUNT.to_owned(),
        label: "Personal".to_owned(),
        created_at: 1_789_000_000,
        counts: holdings(),
        defect: None,
        mismatched,
    }
}

fn open_backup(cx: &mut TestAppContext, name: &str, available: bool) -> World {
    let (fake, state, path) = sdk_fake::start(name);
    fake.enable_stream();
    let fake = Arc::new(fake);
    let backup = state.lock().expect("state").backup.clone();
    backup.set_available(available);
    backup.account(ACCOUNT, holdings());
    let bus = Arc::downgrade(&fake);
    backup.set_publisher(Box::new(move |account, topic, body| {
        if let Some(bus) = bus.upgrade() {
            bus.publish(account, topic, body);
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
    assert!(sent[0]["output_path"].as_str().is_some_and(|p| p.ends_with("/personal.rfbackup")));
    assert!(exists(cx, world.handle, "backup-job"));
    assert!(exists(cx, world.handle, "backup-progress"));
    assert!(exists(cx, world.handle, "backup-job-cancel"));
    assert!(!exists(cx, world.handle, "backup-open"), "one job at a time");

    for _ in 0..4 {
        world.backup.step();
    }
    wait(cx, "writing", |cx| job(cx, &world).is_some_and(|j| j.progress.items_done > 0));
    world.backup.run_out();
    wait(cx, "done", |cx| phase(cx, &world) == Some(BackupPhase::Done));
    advance(cx, 50);
    assert!(exists(cx, world.handle, "backup-result"));
    assert_eq!(job(cx, &world).unwrap().summary.unwrap().counts.messages, 12_000);
    click(cx, world.handle, "backup-job-close");
    assert!(exists(cx, world.handle, "backup-open"), "closed: the forms again");
}

#[gpui_kit::test]
fn a_restore_is_checked_reviewed_then_merged_with_its_attachment_check(cx: &mut TestAppContext) {
    let world = open_backup(cx, "ui-backup-restore", true);
    world.backup.archive("/b/personal.rfbackup", archive(2));
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
    let report = job(cx, &world).unwrap().report.unwrap();
    assert_eq!((report.attachments.verified, report.attachments.mismatched), (318, 2));
}

#[gpui_kit::test]
fn a_wrong_passphrase_is_explained_and_the_file_is_kept_to_try_again(cx: &mut TestAppContext) {
    let world = open_backup(cx, "ui-backup-wrong", true);
    world.backup.archive("/b/personal.rfbackup", archive(0));
    click(cx, world.handle, "restore-open");
    choose_file(cx, &world, "/b/personal.rfbackup");
    set_input(cx, &world, 2, "wrong words entirely");
    click(cx, world.handle, "restore-verify");
    wait(cx, "failed", |cx| phase(cx, &world) == Some(BackupPhase::Failed));
    advance(cx, 50);
    assert_eq!(job(cx, &world).unwrap().failure, Some(BackupFailure::WrongPassphrase));
    assert!(exists(cx, world.handle, "backup-failure"));
    click(cx, world.handle, "backup-job-retry");
    assert_eq!(form(cx, &world), Some(Form::Restore));
    assert_eq!(disabled(cx, &world, "restore-verify"), Some(true), "the passphrase again");
    set_input(cx, &world, 2, PASSPHRASE);
    assert_ne!(disabled(cx, &world, "restore-verify"), Some(true), "the file is kept");

    // A file the daemon cannot read is refused in the form.
    choose_file(cx, &world, "/b/missing.rfbackup");
    set_input(cx, &world, 2, PASSPHRASE);
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
