// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Words of the backup section: passphrase strength, job phases and
//! categories, typed failures, and item counts.

use gpui_kit::SharedString;
use gpui_kit::component::description_list::DescriptionList;
use ramflux_desktop_core::{
    BackupCategory, BackupCounts, BackupFailure, BackupJob, BackupJobKind, BackupPhase,
};

use crate::i18n::{self, backend_text, tr, tr_args};

/// The shortest passphrase a backup accepts.
pub const MIN_PASSPHRASE_CHARS: usize = 12;

/// How hard a backup passphrase is to guess, roughly: length first, then
/// several words or kinds of character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Strength {
    /// Under [`MIN_PASSPHRASE_CHARS`]: not accepted.
    TooShort,
    /// Accepted, but easy to guess.
    Weak,
    /// Reasonable.
    Fair,
    /// Long, or several words.
    Strong,
}

impl Strength {
    /// The strength of `passphrase`.
    pub fn of(passphrase: &str) -> Self {
        let chars = passphrase.chars().count();
        if chars < MIN_PASSPHRASE_CHARS {
            return Self::TooShort;
        }
        // A character outside ASCII (CJK, for one) comes from a far larger
        // set: it counts twice towards length.
        let weight: usize = passphrase.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum();
        let words = passphrase.split_whitespace().filter(|w| w.chars().count() >= 3).count();
        let kinds = [
            passphrase.chars().any(char::is_lowercase),
            passphrase.chars().any(char::is_uppercase),
            passphrase.chars().any(|c| c.is_ascii_digit()),
            passphrase.chars().any(|c| !c.is_alphanumeric() && !c.is_whitespace()),
            !passphrase.is_ascii(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count();
        if weight >= 20 || (words >= 4 && weight >= 16) {
            Self::Strong
        } else if weight >= 16 || words >= 3 || kinds >= 3 {
            Self::Fair
        } else {
            Self::Weak
        }
    }

    /// The hint shown under the passphrase.
    pub fn hint(self) -> SharedString {
        tr(match self {
            Self::TooShort => "settings.backup.strength.too_short",
            Self::Weak => "settings.backup.strength.weak",
            Self::Fair => "settings.backup.strength.fair",
            Self::Strong => "settings.backup.strength.strong",
        })
    }
}

/// What a job is doing, as its heading.
pub fn heading(job: &BackupJob) -> SharedString {
    tr(match (job.kind, job.phase) {
        (BackupJobKind::Backup, BackupPhase::Done) => "settings.backup.done.backup",
        (BackupJobKind::Restore, BackupPhase::Done) => "settings.backup.done.restore",
        (BackupJobKind::Backup, BackupPhase::Failed) => "settings.backup.failed.backup",
        (BackupJobKind::Restore, BackupPhase::Failed) => "settings.backup.failed.restore",
        (BackupJobKind::Backup, BackupPhase::Cancelled) => "settings.backup.cancelled.backup",
        (BackupJobKind::Restore, BackupPhase::Cancelled) => "settings.backup.cancelled.restore",
        (_, BackupPhase::Verified) => "settings.backup.verified.title",
        (_, BackupPhase::Installing) => "settings.backup.job.installing",
        (_, BackupPhase::Verifying) => "settings.backup.job.verifying",
        (_, BackupPhase::Preparing | BackupPhase::Writing) => "settings.backup.job.creating",
    })
}

/// A category's name.
pub fn category(category: BackupCategory) -> SharedString {
    tr(match category {
        BackupCategory::Identity => "settings.backup.category.identity",
        BackupCategory::Contacts => "settings.backup.category.contacts",
        BackupCategory::Conversations => "settings.backup.category.conversations",
        BackupCategory::Messages => "settings.backup.category.messages",
        BackupCategory::Groups => "settings.backup.category.groups",
        BackupCategory::Surfaces => "settings.backup.category.surfaces",
        BackupCategory::Tasks => "settings.backup.category.tasks",
        BackupCategory::Settings => "settings.backup.category.settings",
        BackupCategory::Attachments => "settings.backup.category.attachments",
    })
}

/// A count with thousands separators.
pub fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Why a job failed.
pub fn failure(failure: &BackupFailure) -> SharedString {
    let key = |code: &str| format!("backend.backup.{code}");
    match failure {
        BackupFailure::WrongPassphrase => tr(&key("wrong_passphrase")),
        BackupFailure::Tampered => tr(&key("tampered")),
        BackupFailure::Truncated => tr(&key("truncated")),
        BackupFailure::UnsupportedVersion => tr(&key("unsupported_version")),
        BackupFailure::InsufficientSpace { needed, available } => space(*needed, *available),
        BackupFailure::Unreadable => tr(&key("unreadable")),
        BackupFailure::NotWritable => tr(&key("not_writable")),
        BackupFailure::StagingExpired => tr(&key("staging_expired")),
        BackupFailure::Interrupted => tr(&key("interrupted")),
        BackupFailure::Other(code) => tr_args("settings.backup.error_other", &[("code", code)]),
    }
}

fn space(needed: Option<u64>, available: Option<u64>) -> SharedString {
    match (needed, available) {
        (Some(needed), Some(available)) => tr_args(
            "backend.backup.insufficient_space",
            &[("needed", &i18n::bytes(needed)), ("available", &i18n::bytes(available))],
        ),
        _ => tr("settings.backup.no_space"),
    }
}

/// A refused job request's message (a `backend.backup.*` key, with the
/// space figures in bytes), or another failure's.
pub fn refusal(message: &str) -> SharedString {
    if let Some(args) = message.strip_prefix("backend.backup.insufficient_space ") {
        let figures: serde_json::Value = serde_json::from_str(args).unwrap_or_default();
        let figure = |field: &str| figures[field].as_str().and_then(|n| n.parse::<u64>().ok());
        return space(figure("needed"), figure("available"));
    }
    backend_text(message)
}

/// What an archive holds (or what a restore installed), one row per kind.
pub fn counts(list: DescriptionList, counts: &BackupCounts) -> DescriptionList {
    let files = tr_args(
        "settings.backup.files_value",
        &[("count", &count(counts.attachments)), ("size", &i18n::bytes(counts.attachment_bytes))],
    );
    list.item(
        tr("settings.backup.category.messages"),
        SharedString::from(count(counts.messages)),
        1,
    )
    .item(
        tr("settings.backup.category.conversations"),
        SharedString::from(count(counts.conversations)),
        1,
    )
    .item(tr("settings.backup.category.groups"), SharedString::from(count(counts.groups)), 1)
    .item(tr("settings.backup.category.contacts"), SharedString::from(count(counts.contacts)), 1)
    .item(tr("settings.backup.category.attachments"), files, 1)
    .item(tr("settings.backup.category.surfaces"), SharedString::from(count(counts.surfaces)), 1)
    .item(tr("settings.backup.category.tasks"), SharedString::from(count(counts.tasks)), 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strength_reads_length_then_words_and_kinds() {
        assert_eq!(Strength::of("short"), Strength::TooShort);
        assert_eq!(Strength::of("abcdefghijkl"), Strength::Weak);
        assert_eq!(Strength::of("Abcdefgh1jk!"), Strength::Fair);
        assert_eq!(Strength::of("river stone lantern"), Strength::Fair);
        assert_eq!(Strength::of("river stone lantern harbor"), Strength::Strong);
        // Twelve CJK characters (U+5907) count twice each.
        assert_eq!(Strength::of(&"\u{5907}".repeat(12)), Strength::Strong);
        assert_eq!(Strength::of(&"\u{5907}".repeat(11)), Strength::TooShort);
    }

    #[test]
    fn counts_group_their_thousands() {
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(12_000), "12,000");
        assert_eq!(count(1_234_567), "1,234,567");
    }
}
