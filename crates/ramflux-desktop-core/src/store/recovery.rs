// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The recovery ceremony of a born-v3 account.
//!
//! An update is the whole record. It is validated in full (a phase the
//! recovery may move to, the account, id and expiry unchanged) before
//! anything changes. Neither the words nor the passphrase reach here.

use super::Store;
use super::changes::{ApplyError, Changes};
use crate::model::{RecoveryCeremony, RecoveryPhase};

impl Store {
    pub(super) fn update_recovery(
        &mut self,
        update: &RecoveryCeremony,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        let current = self.recovery.as_ref().filter(|current| current.is_active());
        // With nothing running, only a new recovery or a refused begin (a
        // failure with no id) is news; a late failure of an ended recovery
        // must not overwrite its record.
        let stray = current.is_none()
            && matches!(update.phase, RecoveryPhase::Failed(_))
            && !update.ceremony_id.is_empty();
        if stray || !update.phase.can_follow(current.map(|current| &current.phase)) {
            return Err(ApplyError::IllegalRecoveryTransition {
                from: current.map(|current| current.phase.clone()),
                to: update.phase.clone(),
            });
        }
        if let Some(current) = current
            && !matches!(update.phase, RecoveryPhase::Failed(_) | RecoveryPhase::Cancelled)
            && (current.ceremony_id != update.ceremony_id
                || current.expires_at != update.expires_at
                || current.account_id != update.account_id)
        {
            return Err(ApplyError::RecoveryChanged);
        }
        // One flag for both ceremonies: their panels redraw on it.
        changes.ceremony = true;
        self.recovery = Some(update.clone());
        self.recovery_revision += 1;
        Ok(())
    }
}
