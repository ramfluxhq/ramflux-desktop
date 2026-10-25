// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Backup: a full backup of the open account (everything it
//! holds, every file, no count limits) and a full restore from a backup
//! file (checked completely, reviewed, then installed; merged with the
//! account when it is already here, the newer item kept).
//!
//! The job itself lives in the store (`BackupJob`, from the daemon); this
//! panel holds only what the user is typing and choosing (view-local, R4):
//! which form is open, the chosen file, the masked passphrases (handed to
//! the root as a secret and cleared at once), the jobs it followed and the
//! finished ones the user closed. While a job runs, it is shown instead of
//! the forms: one job at a time.
//!
//! - `forms`: the backup and restore forms.
//! - `job`: a job in progress, the review of a checked backup, and the
//!   result.
//! - `text`: passphrase strength, phases, categories, failures, counts.

mod forms;
mod job;
pub mod text;

use std::collections::BTreeSet;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage};
use gpui_kit::{AnyElement, App, Context, Entity, Subscription, Window};
use ramflux_desktop_core::{
    AccountId, BackupJob, BackupJobId, BackupJobKind, BackupPhase, EffectFailure, Intent,
    IntentKind, LockState, SecretInput, Store, methods, selectors,
};

use crate::app::{StoreEvent, StoreHandle};
use crate::i18n::tr;
use crate::root::IntentRoot;

/// The contract methods of this section.
const BACKUP_METHODS: [&str; 4] = [
    methods::BACKUP_CREATE,
    methods::RESTORE_VERIFY,
    methods::RESTORE_INSTALL,
    methods::BACKUP_CANCEL,
];

/// Which form is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// Create a backup.
    Backup,
    /// Restore from a backup file.
    Restore,
}

/// A part of the section, rendered by its setting item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Backup,
    Restore,
    Job,
}

/// The backup section's view state.
pub struct BackupPanel {
    store: Entity<StoreHandle>,
    root: Entity<IntentRoot>,
    form: Option<Form>,
    /// Where the backup goes.
    location: Option<String>,
    /// The backup file to restore.
    file: Option<String>,
    passphrase: Entity<InputState>,
    repeat: Entity<InputState>,
    restore_secret: Entity<InputState>,
    /// The failed effects counted when a form was submitted: a later
    /// failure of a backup request is shown in the form.
    failures_mark: Option<u64>,
    /// Jobs seen running here, or started from here: their result is
    /// shown when they end.
    followed: BTreeSet<BackupJobId>,
    /// The jobs known when a job was asked for: the next one is its job,
    /// even when it ends at once (a wrong passphrase).
    asked: Option<BTreeSet<BackupJobId>>,
    /// Finished jobs the user closed.
    dismissed: BTreeSet<BackupJobId>,
    _subscriptions: Vec<Subscription>,
}

impl BackupPanel {
    /// The panel over `store`, submitting through `root`.
    pub fn new(
        store: Entity<StoreHandle>,
        root: Entity<IntentRoot>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let masked = |key: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            crate::i18n::localized_input(key, window, cx, |input| input.masked(true))
        };
        let passphrase = masked("settings.backup.passphrase", window, cx);
        let repeat = masked("settings.backup.repeat", window, cx);
        let restore_secret = masked("settings.backup.passphrase", window, cx);
        let mut subscriptions = vec![cx.subscribe(&store, |this, _, event: &StoreEvent, cx| {
            if let StoreEvent::Changed(changes) = event
                && changes.touches_settings()
            {
                this.follow_jobs(cx);
                cx.notify();
            }
        })];
        for input in [&passphrase, &repeat, &restore_secret] {
            subscriptions.push(cx.subscribe(input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }));
        }
        let mut panel = Self {
            store,
            root,
            form: None,
            location: None,
            file: None,
            passphrase,
            repeat,
            restore_secret,
            failures_mark: None,
            followed: BTreeSet::new(),
            asked: None,
            dismissed: BTreeSet::new(),
            _subscriptions: subscriptions,
        };
        panel.follow_jobs(cx);
        panel
    }

    /// The open form.
    pub fn form(&self) -> Option<Form> {
        self.form
    }

    /// The masked inputs: new passphrase, its repetition, restore passphrase.
    pub fn inputs(&self) -> [&Entity<InputState>; 3] {
        [&self.passphrase, &self.repeat, &self.restore_secret]
    }

    /// Follows a running job: its result is shown when it ends, and the
    /// form that started it closes.
    fn follow_jobs(&mut self, cx: &App) {
        let store = self.store.read(cx).store();
        let asked = self.asked.as_ref().and_then(|known| {
            selectors::latest_backup_job(store).filter(|job| !known.contains(&job.id))
        });
        let new = asked
            .or_else(|| selectors::running_backup_job(store))
            .filter(|job| !self.followed.contains(&job.id))
            .map(|job| job.id.clone());
        if let Some(id) = new {
            self.followed.insert(id);
            self.asked = None;
            self.form = None;
            self.failures_mark = None;
        }
    }

    /// The job to show: the running one, else the last one followed here
    /// and not closed.
    fn shown<'a>(&self, store: &'a Store) -> Option<&'a BackupJob> {
        selectors::running_backup_job(store).or_else(|| {
            selectors::latest_backup_job(store)
                .filter(|j| self.followed.contains(&j.id) && !self.dismissed.contains(&j.id))
        })
    }

    fn account(&self, cx: &App) -> Option<AccountId> {
        let handle = self.store.read(cx);
        let store = handle.store();
        handle.active_account().filter(|id| store.account(id).is_some()).cloned().or_else(|| {
            store.accounts().find(|a| a.lock_state == LockState::Unlocked).map(|a| a.id.clone())
        })
    }

    /// The newest failed backup request since the form was submitted.
    fn refused(&self, cx: &App) -> Option<EffectFailure> {
        let mark = self.failures_mark?;
        self.store
            .read(cx)
            .store()
            .failures_since(mark)
            .iter()
            .rev()
            .find(|f| BACKUP_METHODS.contains(&f.method.as_str()))
            .cloned()
    }

    /// Opens `form` (closing the other), cleared.
    pub fn open(&mut self, form: Form, window: &mut Window, cx: &mut Context<Self>) {
        self.form = Some(form);
        self.failures_mark = None;
        for input in [&self.passphrase, &self.repeat, &self.restore_secret] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        cx.notify();
    }

    /// Closes the open form; nothing is submitted.
    pub fn close_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form = None;
        self.failures_mark = None;
        for input in [&self.passphrase, &self.repeat, &self.restore_secret] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        cx.notify();
    }

    /// Sets where the backup goes.
    pub fn set_location(&mut self, path: String, cx: &mut Context<Self>) {
        self.location = Some(path);
        cx.notify();
    }

    /// Sets the backup file to restore.
    pub fn set_file(&mut self, path: String, cx: &mut Context<Self>) {
        self.file = Some(path);
        cx.notify();
    }

    fn take(input: &Entity<InputState>, window: &mut Window, cx: &mut App) -> SecretInput {
        let secret = SecretInput::new(input.read(cx).value().to_string());
        input.update(cx, |input, cx| input.set_value("", window, cx));
        secret
    }

    /// Whether the backup form can start: a location, a long enough
    /// passphrase typed twice the same.
    fn backup_ready(&self, cx: &App) -> bool {
        let typed = self.passphrase.read(cx).value();
        self.location.is_some()
            && text::Strength::of(&typed) > text::Strength::TooShort
            && self.repeat.read(cx).value() == typed
    }

    /// Starts the backup (the passphrase leaves the inputs now).
    pub fn start_backup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(account_id), Some(path)) = (self.account(cx), self.location.clone()) else {
            return;
        };
        if !self.backup_ready(cx) {
            return;
        }
        let secret = Self::take(&self.passphrase, window, cx);
        self.repeat.update(cx, |input, cx| input.set_value("", window, cx));
        self.submit(IntentKind::CreateBackup { account_id, path, secret }, window, cx);
    }

    /// Starts checking the backup file (the passphrase leaves the input now).
    pub fn start_restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.file.clone() else { return };
        let secret = Self::take(&self.restore_secret, window, cx);
        if secret.is_empty() {
            return;
        }
        self.submit(IntentKind::VerifyRestore { path, secret }, window, cx);
    }

    fn submit(&mut self, kind: IntentKind, window: &mut Window, cx: &mut Context<Self>) {
        let store = self.store.read(cx).store();
        self.failures_mark = Some(store.failures_total());
        if matches!(kind, IntentKind::CreateBackup { .. } | IntentKind::VerifyRestore { .. }) {
            self.asked = Some(store.backup_jobs().map(|job| job.id.clone()).collect());
        }
        let root = self.root.clone();
        root.update(cx, |root, cx| root.submit(Intent::human(kind), window, cx));
        cx.notify();
    }

    /// Installs a checked backup the user reviewed.
    pub fn install(&mut self, job_id: BackupJobId, window: &mut Window, cx: &mut Context<Self>) {
        self.submit(IntentKind::InstallRestore { job_id }, window, cx);
    }

    /// Cancels a job (a checked restore's staged copy is discarded).
    pub fn cancel(&mut self, job_id: BackupJobId, window: &mut Window, cx: &mut Context<Self>) {
        self.submit(IntentKind::CancelBackupJob { job_id }, window, cx);
    }

    /// Closes a finished job's result.
    pub fn dismiss(&mut self, job_id: BackupJobId, cx: &mut Context<Self>) {
        self.dismissed.insert(job_id);
        cx.notify();
    }

    /// Closes a failed job's result and opens its form again, with the
    /// same file.
    pub fn retry(&mut self, job: &BackupJob, window: &mut Window, cx: &mut Context<Self>) {
        self.dismissed.insert(job.id.clone());
        let form = match job.kind {
            BackupJobKind::Backup => {
                self.location = Some(job.path.clone());
                Form::Backup
            }
            BackupJobKind::Restore => {
                self.file = Some(job.path.clone());
                Form::Restore
            }
        };
        self.open(form, window, cx);
    }

    fn render_part(
        &mut self,
        part: Part,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match part {
            Part::Backup => self.render_backup(window, cx),
            Part::Restore => self.render_restore(window, cx),
            Part::Job => self.render_job(window, cx),
        }
    }
}

fn item(panel: &Entity<BackupPanel>, part: Part) -> SettingItem {
    let panel = panel.clone();
    SettingItem::render(move |_, window, cx: &mut App| {
        panel.update(cx, |panel, cx| panel.render_part(part, window, cx))
    })
}

/// The backup section.
pub fn page(panel: &Entity<BackupPanel>, cx: &App) -> SettingPage {
    let state = panel.read(cx);
    let store = state.store.read(cx).store();
    let page = SettingPage::new(tr("settings.section.backup"));
    if state.account(cx).is_none() {
        return page.group(SettingGroup::new().item(SettingItem::render(|_, _, cx: &mut App| {
            super::note("settings.account.none", cx)
        })));
    }
    match state.shown(store) {
        Some(job) => {
            page.group(SettingGroup::new().title(text::heading(job)).item(item(panel, Part::Job)))
        }
        None => page
            .group(
                SettingGroup::new()
                    .title(tr("settings.backup.create_title"))
                    .item(item(panel, Part::Backup)),
            )
            .group(
                SettingGroup::new()
                    .title(tr("settings.backup.restore_title"))
                    .item(item(panel, Part::Restore)),
            ),
    }
}

/// Whether `phase` shows progress.
const fn running(phase: BackupPhase) -> bool {
    matches!(
        phase,
        BackupPhase::Preparing
            | BackupPhase::Writing
            | BackupPhase::Verifying
            | BackupPhase::Installing
    )
}
