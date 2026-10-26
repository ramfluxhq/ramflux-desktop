// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Full backup and full restore jobs (BACKUP-01, product
//! `docs/account_backup.md`).
//!
//! The daemon runs one job at a time and reports each change as the whole
//! job with a rising per-job version. A backup first fetches every file the
//! account references but does not hold yet, then writes everything the
//! account holds (identity, contacts, conversations, every message, groups,
//! surfaces, tasks, settings and every attachment) into one encrypted
//! archive, without count limits. A file that stays unavailable fails the
//! backup ([`BackupFailure::AttachmentsUnavailable`]) unless the user
//! explicitly allowed recording it as missing; the archive then lists it in
//! its authenticated omission manifest. A restore first reads and verifies
//! the whole archive into a staging area (`Verifying`): every file must
//! match its hashes, else nothing is installed. It shows what the archive
//! holds (`Verified`, [`RestorePreview`]) and waits for the user; only then
//! is it installed in one step (`Installing`). Installing onto an account
//! that is already on this device merges: where both have an item, the
//! newer one is kept, per category ([`RestoreReport::kept_newer`]).
//!
//! The passphrase never enters a job: it travels with the starting request
//! only.

use serde::{Deserialize, Serialize};

use crate::ids::{AccountId, BackupJobId, Timestamp};

/// What a job does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BackupJobKind {
    /// Write a full backup of an account to a file.
    Backup,
    /// Verify a backup file, then install it.
    Restore,
}

/// Where a job is.
///
/// A backup goes `Preparing` (fetching missing files, then the snapshot) →
/// `Writing` → `Done`; a restore goes `Verifying` → `Verified` →
/// `Installing` → `Done`. Either may end `Failed`; cancelling is possible
/// until a restore starts installing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BackupPhase {
    /// Backup: fetching the files not yet on this device, then taking a
    /// consistent snapshot and counting what it holds.
    Preparing,
    /// Backup: writing the archive.
    Writing,
    /// Restore: reading and verifying the whole archive into staging.
    Verifying,
    /// Restore: the whole archive verified; waiting for the user.
    Verified,
    /// Restore: installing the verified archive (cannot be cancelled).
    Installing,
    /// Finished.
    Done,
    /// Stopped by an error ([`BackupJob::failure`]); nothing partial is kept.
    Failed,
    /// Cancelled by the user; nothing partial is kept.
    Cancelled,
}

impl BackupPhase {
    /// Whether the job has ended.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }

    /// Whether the user may still cancel.
    pub const fn is_cancellable(self) -> bool {
        matches!(self, Self::Preparing | Self::Writing | Self::Verifying | Self::Verified)
    }

    /// Whether `kind` may move from `from` to `to`.
    pub fn can_transition(kind: BackupJobKind, from: Self, to: Self) -> bool {
        use BackupPhase::{
            Cancelled, Done, Failed, Installing, Preparing, Verified, Verifying, Writing,
        };
        match kind {
            BackupJobKind::Backup => matches!(
                (from, to),
                (Preparing, Writing | Failed | Cancelled) | (Writing, Done | Failed | Cancelled)
            ),
            BackupJobKind::Restore => matches!(
                (from, to),
                (Verifying, Verified | Failed | Cancelled)
                    | (Verified, Installing | Failed | Cancelled)
                    | (Installing, Done | Failed)
            ),
        }
    }

    /// The phases a job of `kind` starts in.
    pub fn is_start(kind: BackupJobKind, phase: Self) -> bool {
        match kind {
            BackupJobKind::Backup => {
                !matches!(phase, Self::Verifying | Self::Verified | Self::Installing)
            }
            BackupJobKind::Restore => !matches!(phase, Self::Preparing | Self::Writing),
        }
    }
}

/// What part of the account a job is working on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BackupCategory {
    /// Keys, devices and the recovery manifest (where a job starts).
    #[default]
    Identity,
    /// Contacts, with their restrictions and removals.
    Contacts,
    /// Conversations.
    Conversations,
    /// Direct and group messages.
    Messages,
    /// Groups: members, roles and what this device needs to keep taking part.
    Groups,
    /// Agent surfaces.
    Surfaces,
    /// Conversation tasks.
    Tasks,
    /// Account settings.
    Settings,
    /// Files: every attachment's bytes, keys and metadata.
    Attachments,
}

/// How far a running job is. Totals are `None` until known.
///
/// While a backup is `Preparing` with the category `Attachments` and an
/// item total, it is fetching the files not yet on this device: the items
/// are the files tried so far ([`BackupProgress::is_fetching`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupProgress {
    /// The part being worked on.
    #[serde(default)]
    pub category: BackupCategory,
    /// Items done (of every category).
    #[serde(default)]
    pub items_done: u64,
    /// Items in all, when known.
    #[serde(default)]
    pub items_total: Option<u64>,
    /// Bytes done (archive bytes written or read).
    #[serde(default)]
    pub bytes_done: u64,
    /// Bytes in all, when known.
    #[serde(default)]
    pub bytes_total: Option<u64>,
}

impl BackupProgress {
    /// Whether a backup in `phase` is fetching missing files before its
    /// snapshot.
    pub fn is_fetching(&self, phase: BackupPhase) -> bool {
        phase == BackupPhase::Preparing
            && self.category == BackupCategory::Attachments
            && self.items_total.is_some()
    }

    /// The fraction done, 0 to 1, by bytes when their total is known, else
    /// by items; `None` when neither total is known.
    pub fn fraction(&self) -> Option<f32> {
        let ratio = |done: u64, total: u64| {
            // Lossy by design: shown as a bar.
            #[allow(clippy::cast_precision_loss)]
            let value = if total == 0 { 1.0 } else { done as f64 / total as f64 };
            #[allow(clippy::cast_possible_truncation)]
            let value = value.clamp(0.0, 1.0) as f32;
            value
        };
        match (self.bytes_total, self.items_total) {
            (Some(total), _) => Some(ratio(self.bytes_done, total)),
            (None, Some(total)) => Some(ratio(self.items_done, total)),
            (None, None) => None,
        }
    }
}

/// How many of each kind of item.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupCounts {
    /// Contacts.
    #[serde(default)]
    pub contacts: u64,
    /// Conversations.
    #[serde(default)]
    pub conversations: u64,
    /// Messages (direct and group).
    #[serde(default)]
    pub messages: u64,
    /// Groups.
    #[serde(default)]
    pub groups: u64,
    /// Agent surfaces.
    #[serde(default)]
    pub surfaces: u64,
    /// Conversation tasks.
    #[serde(default)]
    pub tasks: u64,
    /// Attachments (files).
    #[serde(default)]
    pub attachments: u64,
    /// The attachments' bytes.
    #[serde(default)]
    pub attachment_bytes: u64,
}

impl BackupCounts {
    /// Items of every kind (files included, their bytes not).
    pub const fn items(&self) -> u64 {
        self.contacts
            .saturating_add(self.conversations)
            .saturating_add(self.messages)
            .saturating_add(self.groups)
            .saturating_add(self.surfaces)
            .saturating_add(self.tasks)
            .saturating_add(self.attachments)
    }
}

/// What a finished backup wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSummary {
    /// What the archive holds.
    pub counts: BackupCounts,
    /// The archive's size in bytes.
    pub file_bytes: u64,
}

/// What a verified archive holds, shown before it is installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestorePreview {
    /// The account in the archive (its local id).
    pub account_id: AccountId,
    /// The account's name, when the archive carries one.
    #[serde(default)]
    pub account_label: Option<String>,
    /// When the backup was made.
    pub created_at: Timestamp,
    /// What the archive holds.
    pub counts: BackupCounts,
    /// The archive's size in bytes.
    pub file_bytes: u64,
    /// The account is already on this device: installing merges (the newer
    /// of two versions of an item is kept). Otherwise the account is added.
    pub merge: bool,
    /// The file check of the verification (every file the archive holds
    /// matched; the ones the backup recorded as unavailable at its source
    /// are counted apart).
    #[serde(default)]
    pub attachments: AttachmentCheck,
}

/// The attachment check of a restore: every file's bytes against the
/// hashes the archive recorded.
///
/// A restore installs only when every file the archive holds matched and
/// every missing file is one the backup explicitly recorded as unavailable
/// at its source ([`AttachmentCheck::all_verified`]); anything else fails it
/// ([`BackupFailure::AttachmentMismatch`]) and nothing is installed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentCheck {
    /// Files whose bytes match their hashes.
    pub verified: u64,
    /// Files whose bytes do not match.
    pub mismatched: u64,
    /// Files without their bytes; includes `unavailable_at_source`.
    pub missing: u64,
    /// Missing files the backup explicitly recorded as unavailable when it
    /// was made (the user allowed it), with their expected hashes.
    #[serde(default)]
    pub unavailable_at_source: u64,
    /// Merging: files this device referenced but had not downloaded yet,
    /// kept as they were (still fetchable later); not archive omissions.
    #[serde(default)]
    pub local_missing_kept: u64,
}

impl AttachmentCheck {
    /// Whether every file the archive holds matched, and every missing file
    /// is one the backup recorded as unavailable at its source.
    pub const fn all_verified(&self) -> bool {
        self.mismatched == 0 && self.missing == self.unavailable_at_source
    }

    /// Missing files the backup did not record as unavailable at its source.
    pub const fn unapproved_missing(&self) -> u64 {
        self.missing.saturating_sub(self.unavailable_at_source)
    }
}

/// What a finished restore installed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreReport {
    /// Items installed from the archive, per kind.
    pub restored: BackupCounts,
    /// Items this device already had in a newer version, which were kept,
    /// per kind.
    #[serde(default)]
    pub kept_newer: BackupCounts,
    /// The attachment check.
    pub attachments: AttachmentCheck,
}

/// Why a job failed: the daemon's typed codes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackupFailure {
    /// The passphrase does not open the archive.
    WrongPassphrase,
    /// The archive was changed or damaged (authentication failed).
    Tampered,
    /// The archive ends early (cut short while copying or writing).
    Truncated,
    /// Made by a newer format version this daemon cannot read.
    UnsupportedVersion,
    /// Not enough disk space (in bytes, when the daemon says).
    InsufficientSpace {
        /// Space needed.
        needed: Option<u64>,
        /// Space available.
        available: Option<u64>,
    },
    /// The backup file cannot be read (missing or no permission).
    Unreadable,
    /// The destination cannot be written.
    NotWritable,
    /// The verified staging copy expired before it was installed.
    StagingExpired,
    /// The daemon stopped while the job ran; nothing was changed.
    Interrupted,
    /// Restore: a file the archive holds does not match its hashes, or is
    /// missing without having been recorded as unavailable at the source;
    /// nothing was installed. The check counts, when the daemon sent them.
    AttachmentMismatch {
        /// The file check as far as it went.
        checks: Option<AttachmentCheck>,
    },
    /// Backup: files this account references could not be fetched. Nothing
    /// was written; the backup may be made again with them explicitly
    /// recorded as unavailable.
    AttachmentsUnavailable {
        /// How many files.
        count: u64,
        /// The first page of their (opaque) selectors.
        selectors: Vec<String>,
        /// Where the next page starts (`account.backup.jobs {job_id,
        /// missing_after}`), when there is one.
        next_cursor: Option<String>,
    },
    /// Restore: the archive would roll the account's identity back to an
    /// older epoch (a new install only; a merge keeps the live identity).
    IdentityRollback,
    /// Restore: the archive's identity root differs from the account's here;
    /// a root is never replaced.
    RootConflict,
    /// Another code (kept as the daemon wrote it).
    Other(String),
}

/// One full backup or restore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupJob {
    /// The daemon's job id.
    pub id: BackupJobId,
    /// Backup or restore.
    pub kind: BackupJobKind,
    /// The file written (backup) or read (restore).
    pub path: String,
    /// Where it is.
    pub phase: BackupPhase,
    /// The daemon's per-job version; rises with every change.
    pub version: u64,
    /// When it started.
    pub started_at: Timestamp,
    /// When it ended.
    #[serde(default)]
    pub finished_at: Option<Timestamp>,
    /// How far it is.
    #[serde(default)]
    pub progress: BackupProgress,
    /// A finished backup's summary.
    #[serde(default)]
    pub summary: Option<BackupSummary>,
    /// A verified restore's content (kept while installing and after).
    #[serde(default)]
    pub preview: Option<RestorePreview>,
    /// A finished restore's report.
    #[serde(default)]
    pub report: Option<RestoreReport>,
    /// Why it failed.
    #[serde(default)]
    pub failure: Option<BackupFailure>,
}
