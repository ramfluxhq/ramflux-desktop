// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The creation ceremony of a born-v3 account.
//!
//! An update is the whole ceremony. It is validated in full (a phase the
//! ceremony may move to, the ceremony_id and expiry unchanged) before anything
//! changes. The recovery phrase is not part of it and never reaches here.

use super::Store;
use super::changes::{ApplyError, Changes};
use crate::model::{Ceremony, CeremonyPhase};

impl Store {
    pub(super) fn update_ceremony(
        &mut self,
        update: &Ceremony,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        let current = self.ceremony.as_ref().filter(|current| current.is_active());
        // With no ceremony running, only a new one or a refused begin (a
        // failure with no ceremony_id) is news; a late failure of an ended
        // ceremony must not overwrite its record.
        let stray = current.is_none()
            && matches!(update.phase, CeremonyPhase::Failed(_))
            && !update.ceremony_id.is_empty();
        if stray || !update.phase.can_follow(current.map(|current| &current.phase)) {
            return Err(ApplyError::IllegalCeremonyTransition {
                from: current.map(|current| current.phase.clone()),
                to: update.phase.clone(),
            });
        }
        if let Some(current) = current
            && !matches!(update.phase, CeremonyPhase::Failed(_) | CeremonyPhase::Cancelled)
            && (current.ceremony_id != update.ceremony_id
                || current.expires_at != update.expires_at)
        {
            return Err(ApplyError::CeremonyChanged);
        }
        changes.ceremony = true;
        self.ceremony = Some(update.clone());
        self.ceremony_revision += 1;
        Ok(())
    }
}
