// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Full backup and restore jobs.
//!
//! A job update is the whole job with a rising version. It is validated in
//! full (rising version, same kind and file, a phase the kind starts in or
//! may move to) before anything changes.

use super::Store;
use super::changes::{ApplyError, Changes};
use crate::model::{BackupJob, BackupPhase};

impl Store {
    pub(super) fn upsert_backup_job(
        &mut self,
        job: &BackupJob,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        match self.backup_jobs.get(&job.id) {
            Some(current) => {
                if job.version <= current.version {
                    return Err(ApplyError::StaleBackupVersion {
                        job: job.id.clone(),
                        current: current.version,
                        found: job.version,
                    });
                }
                if job.kind != current.kind || job.path != current.path {
                    return Err(ApplyError::BackupJobChanged(job.id.clone()));
                }
                if job.phase != current.phase
                    && !BackupPhase::can_transition(job.kind, current.phase, job.phase)
                {
                    return Err(ApplyError::IllegalBackupTransition {
                        job: job.id.clone(),
                        kind: job.kind,
                        from: current.phase,
                        to: job.phase,
                    });
                }
            }
            None if !BackupPhase::is_start(job.kind, job.phase) => {
                return Err(ApplyError::IllegalBackupTransition {
                    job: job.id.clone(),
                    kind: job.kind,
                    from: job.phase,
                    to: job.phase,
                });
            }
            None => {}
        }
        changes.backup_jobs.insert(job.id.clone());
        self.backup_jobs.insert(job.id.clone(), job.clone());
        Ok(())
    }
}
