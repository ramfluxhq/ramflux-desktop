// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for full backup and restore. Every decision is human only, and
//! one job runs at a time: a new backup or restore waits until the running
//! one has ended. A restore installs only after it verified completely and
//! the user reviewed what it holds; it can be cancelled until it installs.

use serde_json::json;

use super::{Decision, IntentError, Policy};
use crate::ids::BackupJobId;
use crate::intent::{IntentKind, methods};
use crate::model::{BackupJob, BackupJobKind, BackupPhase};
use crate::selectors;

/// Decides backup-area kinds; `None` for kinds of other areas.
pub(super) fn decide(p: &Policy<'_>, kind: &IntentKind) -> Option<Result<Decision, IntentError>> {
    Some(match kind {
        IntentKind::CreateBackup { account_id, path, secret } => {
            p.human_only("back up an account").and_then(|()| {
                if p.store.account(account_id).is_none() {
                    return Err(IntentError::UnknownAccount(account_id.clone()));
                }
                start(p, path, secret.is_empty(), "backup passphrase")?;
                Ok(Decision::SecretCall(
                    methods::BACKUP_CREATE,
                    json!({ "account_id": account_id, "path": path }),
                ))
            })
        }
        IntentKind::VerifyRestore { path, secret } => {
            p.human_only("restore a backup").and_then(|()| {
                start(p, path, secret.is_empty(), "backup passphrase")?;
                Ok(Decision::SecretCall(methods::RESTORE_VERIFY, json!({ "path": path })))
            })
        }
        IntentKind::InstallRestore { job_id } => p.human_only("restore a backup").and_then(|()| {
            let job = job(p, job_id)?;
            if job.kind != BackupJobKind::Restore || job.phase != BackupPhase::Verified {
                return Err(IntentError::BackupJobPhase(job_id.clone()));
            }
            Ok(Decision::Call(methods::RESTORE_INSTALL, json!({ "job_id": job_id })))
        }),
        IntentKind::CancelBackupJob { job_id } => {
            p.human_only("cancel a backup or restore").and_then(|()| {
                if !job(p, job_id)?.phase.is_cancellable() {
                    return Err(IntentError::BackupJobPhase(job_id.clone()));
                }
                Ok(Decision::Call(methods::BACKUP_CANCEL, json!({ "job_id": job_id })))
            })
        }
        _ => return None,
    })
}

/// Checks a new job: a path, a passphrase, and no job running.
fn start(
    p: &Policy<'_>,
    path: &str,
    secret_empty: bool,
    what: &'static str,
) -> Result<(), IntentError> {
    if path.trim().is_empty() {
        return Err(IntentError::Empty("backup path"));
    }
    if secret_empty {
        return Err(IntentError::Empty(what));
    }
    if selectors::running_backup_job(p.store).is_some() {
        return Err(IntentError::BackupInProgress);
    }
    Ok(())
}

fn job<'a>(p: &Policy<'a>, id: &BackupJobId) -> Result<&'a BackupJob, IntentError> {
    p.store.backup_job(id).ok_or_else(|| IntentError::UnknownBackupJob(id.clone()))
}
