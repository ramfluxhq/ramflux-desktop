// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Offscreen light and dark screenshots of Settings · Backup on a fake
//! daemon with the BACKUP-01 job API as merged: the section, the backup form,
//! a backup in progress and done, the restore form, a backup being checked,
//! the review before restoring (merge, newer wins, the files it omits), restoring, the result
//! with its attachment check, and a wrong passphrase; then, on a fresh
//! device, the entry page finishing an installed restore while the
//! reopened account's first jobs listing is awaited.
//!
//! Not part of `cargo test`; run explicitly (`RAMFLUX_SHOTS_LOCALE` picks
//! the interface language, `en` by default). `RAMFLUX_SHOTS_SET` is
//! required: `restore-hold` takes the entry page's finishing state,
//! `settings` the settings scenario:
//!
//! ```sh
//! RAMFLUX_SHOTS_SET=restore-hold RAMFLUX_SHOTS_DIR=/some/dir cargo test -p ramflux-desktop-ui --test backup_shots
//! ```

mod common;
mod sdk_fake;

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    shots::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("backup shots: skipped; GPUI has no headless renderer on this platform");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod shots {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use gpui_kit::base::Root;
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Bounds, Entity, HeadlessAppContext, Point, Window, WindowBounds,
        WindowHandle, WindowOptions, px, size,
    };
    use ramflux_desktop_core::{BackupPhase, LockState, Store, methods, selectors};
    use ramflux_desktop_test::fake_backup::{Archive, Counts, Defect, FakeBackup};
    use ramflux_desktop_ui::AppAssets as Assets;
    use ramflux_desktop_ui::Shell;
    use ramflux_desktop_ui::route::{EntryStep, Page};
    use ramflux_desktop_ui::settings::SettingsSection;

    use crate::sdk_fake;

    const PASSPHRASE: &str = "river stone lantern harbor";

    struct Run {
        cx: HeadlessAppContext,
        handle: WindowHandle<Root>,
        shell: Entity<Shell>,
        out: PathBuf,
        locale: String,
    }

    impl Run {
        /// A headless window with the production shell on `options`.
        fn open(
            mut cx: HeadlessAppContext,
            options: ramflux_desktop_ui::shell::ShellOptions,
            out: PathBuf,
            locale: String,
            height: f32,
        ) -> Self {
            let (handle, shell) = cx
                .update(|cx| {
                    let bounds =
                        Bounds { origin: Point::default(), size: size(px(1280.), px(height)) };
                    gpui_kit::open_window(
                        WindowOptions {
                            window_bounds: Some(WindowBounds::Windowed(bounds)),
                            focus: false,
                            show: false,
                            ..Default::default()
                        },
                        cx,
                        |window, cx| cx.new(|cx| Shell::with_options(options, window, cx)),
                    )
                })
                .expect("offscreen window");
            let handle = handle.downcast::<Root>().expect("Root");
            Self { cx, handle, shell, out, locale }
        }

        fn click_in(&mut self, scope: &str, id: &'static str) {
            let scope = scope.to_owned();
            self.window(move |window, cx| window.within(scope).click(id, cx));
        }

        fn tick(&mut self) {
            std::thread::sleep(Duration::from_millis(10));
            self.cx.advance_clock(Duration::from_millis(50));
            self.cx.run_until_parked();
        }

        fn settle(&mut self) {
            for _ in 0..10 {
                self.tick();
            }
        }

        fn until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
            let limit = Instant::now() + Duration::from_secs(30);
            while !done(self) {
                assert!(Instant::now() < limit, "timed out waiting for {what}");
                self.tick();
            }
        }

        fn window<R>(&mut self, f: impl FnOnce(&mut Window, &mut gpui_kit::App) -> R) -> R {
            let result = self
                .cx
                .update_window(self.handle.into(), |_, window, cx| {
                    window.render_frame(cx);
                    f(window, cx)
                })
                .expect("window");
            self.cx.run_until_parked();
            result
        }

        fn click(&mut self, id: &str) {
            let id = id.to_owned();
            self.window(move |window, cx| window.click(id, cx));
        }

        fn store<R>(&mut self, f: impl FnOnce(&Store) -> R) -> R {
            self.cx.update(|cx| f(self.shell.read(cx).store().read(cx).store()))
        }

        fn phase(&mut self) -> Option<BackupPhase> {
            self.store(|s| selectors::latest_backup_job(s).map(|j| j.phase))
        }

        fn wait_phase(&mut self, phase: BackupPhase) {
            self.until("the job phase", |r| r.phase() == Some(phase));
            self.settle();
        }

        /// Sets one of the backup panel's masked inputs (0 new, 1 repeat,
        /// 2 restore).
        fn set_input(&mut self, index: usize, value: &'static str) {
            let panel =
                self.cx.update(|cx| self.shell.read(cx).settings().read(cx).backup_panel().clone());
            self.window(move |window, cx| {
                let input = panel.read(cx).inputs()[index].clone();
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            });
        }

        fn panel(&mut self) -> Entity<ramflux_desktop_ui::settings::backup::BackupPanel> {
            self.cx.update(|cx| self.shell.read(cx).settings().read(cx).backup_panel().clone())
        }

        /// Stands in for the native save dialog (headless has none).
        fn set_location(&mut self, path: &str) {
            let panel = self.panel();
            let path = path.to_owned();
            self.cx.update(|cx| panel.update(cx, |panel, cx| panel.set_location(path, cx)));
        }

        /// Stands in for the native open dialog.
        fn set_file(&mut self, path: &str) {
            let panel = self.panel();
            let path = path.to_owned();
            self.cx.update(|cx| panel.update(cx, |panel, cx| panel.set_file(path, cx)));
        }

        fn shot(&mut self, name: &str) {
            for (mode, suffix) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
                self.cx.update(|cx| {
                    Theme::change(mode, None, cx);
                    cx.refresh_windows();
                });
                self.window(|_, _| {});
                let image = self.cx.capture_screenshot(self.handle.into()).expect("render");
                let path = self.out.join(format!("{name}_{}_{suffix}.png", self.locale));
                image.save(&path).expect("write png");
                println!("wrote {}", path.display());
            }
            self.cx.update(|cx| {
                Theme::change(ThemeMode::Light, None, cx);
                cx.refresh_windows();
            });
            self.cx.run_until_parked();
        }
    }

    fn holdings() -> Counts {
        Counts {
            contacts: 1_204,
            conversations: 236,
            messages: 48_210,
            groups: 212,
            surfaces: 31,
            tasks: 17,
            attachments: 1_318,
            attachment_bytes: 3_650_000_000,
        }
    }

    fn archive(defect: Option<Defect>, mismatched: u64) -> Archive {
        Archive {
            created_at: 1_792_900_000,
            defect,
            mismatched,
            ..Archive::new(PASSPHRASE, "acct-main", "Personal", holdings())
        }
    }

    fn backup_fake(fake: &Arc<ramflux_desktop_test::fake_bus::FakeBus>, backup: &FakeBackup) {
        backup.account("acct-main", holdings());
        // The daemon fails a restore whose files do not match their hashes,
        // so the one that reaches its review is intact; it records three
        // files its backup could not read, and this device holds five the
        // archive lacks.
        let recorded = Archive { unavailable_at_source: 3, ..archive(None, 0) };
        backup.archive("/Users/me/Documents/ramflux-backup-2026-10-20.rfbackup", recorded);
        backup.local_pending("acct-main", 5);
        let bus = Arc::downgrade(fake);
        backup.set_publisher(Box::new(move |row| {
            if let Some(bus) = bus.upgrade() {
                bus.publish_job(row);
            }
        }));
    }

    fn headless(locale: &str) -> HeadlessAppContext {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            ramflux_desktop_ui::init(cx);
            ramflux_desktop_ui::i18n::select_locale(locale, cx);
        });
        cx
    }

    /// What the daemon lists for the restored account.
    fn set_account(state: &sdk_fake::Shared, lock_state: &str) {
        let open = lock_state == "unlocked";
        state.lock().expect("state").accounts = Some(serde_json::json!({
            "accounts": if open { vec!["acct-main"] } else { vec![] },
            "account_details": [{ "id": "acct-main", "label": "Personal", "kind": "human",
                                  "state": lock_state }],
        }));
    }

    /// A fresh device whose full restore was checked on the entry page and
    /// installed: the daemon reopens the account, and the account session's
    /// first jobs listing is held back, so the page shows "Finishing the
    /// restore" with its spinner for as long as the shot takes.
    fn restore_hold(out: &std::path::Path, locale: &str) {
        let (fake, state, socket) = sdk_fake::start(&format!("f22-hold-shots-{locale}"));
        fake.enable_stream();
        let fake = Arc::new(fake);
        let backup = state.lock().expect("state").backup.clone();
        backup.archive("/Users/me/Documents/ramflux-backup-2026-10-20.rfbackup", archive(None, 0));
        let bus = Arc::downgrade(&fake);
        backup.set_publisher(Box::new(move |row| {
            if let Some(bus) = bus.upgrade() {
                bus.publish_job(row);
            }
        }));
        state.lock().expect("state").accounts =
            Some(serde_json::json!({ "accounts": [], "account_details": [] }));
        let mut options = sdk_fake::options(&socket);
        if let ramflux_desktop_ui::shell::SourceChoice::Rfd(config) = &mut options.source {
            config.deadlines = ramflux_desktop_sdk::Deadlines::uniform(Duration::from_secs(120));
        }
        let mut run =
            Run::open(headless(locale), options, out.to_path_buf(), locale.to_owned(), 1500.);
        run.until("the entry page", |r| {
            matches!(r.cx.update(|cx| r.shell.read(cx).page()), Page::Entry(_))
        });
        if run.cx.update(|cx| run.shell.read(cx).entry().read(cx).step(cx)) == EntryStep::Node {
            run.click_in("node-n-harbor", "node-confirm");
        }
        run.until("the account step", |r| {
            r.cx.update(|cx| r.shell.read(cx).page()) == Page::Entry(EntryStep::Account)
        });
        run.until("the job API at daemon scope", |r| {
            r.store(|s| s.unsupported_reason(methods::RESTORE_VERIFY).is_none())
        });
        run.click("entry-path-2");
        let panel = run.cx.update(|cx| run.shell.read(cx).entry().read(cx).full_restore().clone());
        run.cx.update(|cx| {
            panel.update(cx, |panel, cx| {
                panel.set_file("/Users/me/Documents/ramflux-backup-2026-10-20.rfbackup".into(), cx);
            });
        });
        run.settle();
        let secret = panel.clone();
        run.window(move |window, cx| {
            let input = secret.read(cx).inputs()[2].clone();
            input.update(cx, |input, cx| input.set_value(PASSPHRASE, window, cx));
        });
        run.click("restore-verify");
        run.wait_phase(BackupPhase::Verifying);
        backup.run_out();
        run.wait_phase(BackupPhase::Verified);
        // The daemon installs, reopens the account, unlocked.
        {
            let state = state.lock().expect("state");
            state.account_jobs.close();
        }
        set_account(&state, "unlocked");
        run.click("restore-install");
        backup.run_out();
        run.until("the account's own session", |r| {
            r.cx.update(|cx| r.shell.read(cx).store().read(cx).active_account().is_some())
        });
        run.settle();
        run.shot("f22_13_restore_finishing");
        state.lock().expect("state").account_jobs.open();
        run.until("the jobs listed", |r| {
            r.store(|s| s.is_listed(ramflux_desktop_core::Listing::BackupJobs))
        });
        // The account locks and opens again, and its new session never
        // answers: past the bound the page says so and offers a way out.
        set_account(&state, "locked");
        run.until("locked", |r| {
            r.store(|s| s.accounts().next().map(|a| a.lock_state) != Some(LockState::Unlocked))
        });
        let reads = state.lock().expect("state").account_reads.clone();
        reads.close();
        set_account(&state, "unlocked");
        run.until("the session never loads", |r| {
            r.cx.update(|cx| r.shell.read(cx).entry().read(cx).session_failed(cx))
        });
        run.settle();
        run.shot("f22_14_session_failed");
        reads.open();
        if let Ok(fake) = Arc::try_unwrap(fake) {
            fake.shutdown();
        }
    }

    pub fn run() {
        let out = std::env::var_os("RAMFLUX_SHOTS_DIR").map_or_else(
            || PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("backup-shots"),
            PathBuf::from,
        );
        let locale = std::env::var("RAMFLUX_SHOTS_LOCALE").unwrap_or_else(|_| "en".to_owned());
        std::fs::create_dir_all(&out).expect("output directory");
        let set = std::env::var("RAMFLUX_SHOTS_SET").unwrap_or_default();
        match set.as_str() {
            "restore-hold" => {
                restore_hold(&out, &locale);
                return;
            }
            // The settings backup scenario (f22_01 to f22_12).
            "settings" => {}
            _ => {
                println!(
                    "skipped: pick RAMFLUX_SHOTS_SET=restore-hold (the entry page finishing a \
                     restore) or =settings (the settings scenario)"
                );
                return;
            }
        }
        let (fake, state, socket) = sdk_fake::start(&format!("f22-shots-{locale}"));
        fake.enable_stream();
        let fake = Arc::new(fake);
        let backup = state.lock().expect("state").backup.clone();
        backup_fake(&fake, &backup);
        let cx = headless(&locale);
        let options = sdk_fake::options(&socket);
        let mut run = Run::open(cx, options, out.clone(), locale.clone(), 1100.);
        run.until("the workspace", |r| {
            r.cx.update(|cx| r.shell.read(cx).page()) == Page::Workspace
        });
        run.until("the job API", |r| {
            r.store(|s| s.unsupported_reason(methods::BACKUP_CREATE).is_none())
        });
        let shell = run.shell.clone();
        run.cx.update(|cx| {
            shell.update(cx, |shell, cx| shell.open_settings_at(SettingsSection::Backup, cx));
        });
        run.settle();
        run.shot("f22_01_backup_section");

        // A full backup.
        run.click("backup-open");
        run.set_location("/Users/me/Documents/ramflux-backup-2026-10-25.rfbackup");
        run.settle();
        run.set_input(0, PASSPHRASE);
        run.set_input(1, "river stone lantern");
        run.settle();
        run.shot("f22_02_backup_form_mismatch");
        run.set_input(1, PASSPHRASE);
        run.settle();
        run.shot("f22_03_backup_form_ready");
        run.click("backup-start");
        run.wait_phase(BackupPhase::Preparing);
        for _ in 0..4 {
            backup.step();
        }
        // Four steps: the files are checked (none are missing), then three
        // categories are written, so messages are next.
        run.until("writing messages", |r| {
            r.store(|s| {
                selectors::latest_backup_job(s)
                    .is_some_and(|j| j.phase == BackupPhase::Writing && j.progress.bytes_done > 0)
            })
        });
        run.settle();
        run.shot("f22_04_backup_progress");
        backup.run_out();
        run.wait_phase(BackupPhase::Done);
        run.shot("f22_05_backup_done");
        run.click("backup-job-close");
        run.settle();

        // A restore that merges, with the files its backup could not read.
        run.click("restore-open");
        run.set_file("/Users/me/Documents/ramflux-backup-2026-10-20.rfbackup");
        run.settle();
        run.set_input(2, PASSPHRASE);
        run.settle();
        run.shot("f22_06_restore_form");
        run.click("restore-verify");
        run.wait_phase(BackupPhase::Verifying);
        for _ in 0..5 {
            backup.step();
        }
        run.settle();
        run.shot("f22_07_restore_checking");
        backup.run_out();
        run.wait_phase(BackupPhase::Verified);
        run.shot("f22_08_restore_review");
        run.click("restore-install");
        run.wait_phase(BackupPhase::Installing);
        for _ in 0..3 {
            backup.step();
        }
        run.settle();
        run.shot("f22_09_restoring");
        backup.run_out();
        run.wait_phase(BackupPhase::Done);
        run.shot("f22_10_restored");
        run.click("backup-job-close");
        run.settle();

        // A wrong passphrase, then a file that was changed.
        run.click("restore-open");
        run.set_file("/Users/me/Documents/ramflux-backup-2026-10-20.rfbackup");
        run.settle();
        run.set_input(2, "river stone lantern");
        run.click("restore-verify");
        run.wait_phase(BackupPhase::Verifying);
        backup.step();
        run.wait_phase(BackupPhase::Failed);
        run.shot("f22_11_wrong_passphrase");
        run.click("backup-job-close");
        backup.archive("/Users/me/Downloads/copied.rfbackup", archive(Some(Defect::Truncated), 0));
        run.click("restore-open");
        run.set_file("/Users/me/Downloads/copied.rfbackup");
        run.settle();
        run.set_input(2, PASSPHRASE);
        run.click("restore-verify");
        run.wait_phase(BackupPhase::Verifying);
        backup.run_out();
        run.wait_phase(BackupPhase::Failed);
        run.shot("f22_12_truncated");

        if let Ok(fake) = Arc::try_unwrap(fake) {
            fake.shutdown();
        }
        drop(run);
        restore_hold(&out, &locale);
    }
}
