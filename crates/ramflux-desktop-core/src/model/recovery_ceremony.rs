// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The recovery ceremony of a born-v3 account (DESK-V3-02): the user types
//! the 24 words of an account that already lives on this device to get its
//! primary credential back.
//!
//! Like the creation ceremony, only what is safe to keep lives here: the
//! phase, the ceremony id, the expiry, the public account fingerprint the
//! user must confirm. The words the user types and the passphrase never
//! enter the store: they travel in one [`crate::Effect::SecretCall`] and are
//! zeroized after it.
//!
//! ```text
//! (none | ended) -> Prepared -> Recovered
//! Prepared -> Failed | Cancelled
//! (none | ended) -> Failed            (the daemon refused to begin)
//! ```
//!
//! A recovery starts only from an explicit user request
//! ([`crate::IntentKind::BeginRecovery`]); nothing a network event carries
//! may open it.

use serde::{Deserialize, Serialize};

use super::{CeremonyAccount, CeremonyFailure};
use crate::ids::{AccountId, Timestamp};

/// Where a recovery is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryPhase {
    /// The daemon prepared the recovery and named the account fingerprint;
    /// it waits for the fingerprint, the words and the passphrase.
    Prepared,
    /// The account's credential was restored and the account is unlocked.
    Recovered(CeremonyAccount),
    /// The recovery ended without a change.
    Failed(CeremonyFailure),
    /// The user cancelled.
    Cancelled,
}

impl RecoveryPhase {
    /// Whether the recovery is over (a new one may begin).
    pub fn is_ended(&self) -> bool {
        !matches!(self, Self::Prepared)
    }

    /// Whether a recovery in `from` (none when there is none) may move to
    /// `self`.
    pub fn can_follow(&self, from: Option<&Self>) -> bool {
        match from.filter(|from| !from.is_ended()) {
            None => matches!(self, Self::Prepared | Self::Failed(_)),
            Some(_) => !matches!(self, Self::Prepared),
        }
    }
}

/// A recovery as the daemon reports it (the whole record).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryCeremony {
    /// Where it is.
    pub phase: RecoveryPhase,
    /// The account being recovered.
    pub account_id: AccountId,
    /// The public fingerprint of the account (its anchor): the user compares
    /// it with the one they wrote down. Empty when the daemon refused to
    /// begin.
    pub anchor: String,
    /// The id of this recovery; empty when the daemon refused to begin.
    pub ceremony_id: String,
    /// When it expires. Stays the same for the whole recovery.
    pub expires_at: Timestamp,
}

impl RecoveryCeremony {
    /// Whether the recovery is still running.
    pub fn is_active(&self) -> bool {
        !self.phase.is_ended()
    }

    /// Whether the time is over at `now`.
    pub fn expired_at(&self, now: Timestamp) -> bool {
        now >= self.expires_at
    }

    /// Milliseconds left at `now` (zero once expired).
    pub fn remaining_ms(&self, now: Timestamp) -> u64 {
        self.expires_at.saturating_since(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Networking;

    fn done() -> RecoveryPhase {
        RecoveryPhase::Recovered(CeremonyAccount {
            local_account_id: "a".into(),
            display_name: "A".into(),
            networking: Networking::Pending,
        })
    }

    #[test]
    fn a_recovery_prepares_then_ends_once() {
        assert!(RecoveryPhase::Prepared.can_follow(None));
        assert!(done().can_follow(Some(&RecoveryPhase::Prepared)));
        assert!(RecoveryPhase::Cancelled.can_follow(Some(&RecoveryPhase::Prepared)));
        assert!(
            RecoveryPhase::Failed(CeremonyFailure::Expired)
                .can_follow(Some(&RecoveryPhase::Prepared))
        );
        // No recovery without a begin, and no second answer to an ended one.
        assert!(!done().can_follow(None));
        assert!(!RecoveryPhase::Cancelled.can_follow(None));
        assert!(!done().can_follow(Some(&RecoveryPhase::Cancelled)));
        assert!(!RecoveryPhase::Prepared.can_follow(Some(&RecoveryPhase::Prepared)));
    }

    #[test]
    fn a_refused_begin_is_a_failure_with_nothing_before_it() {
        assert!(RecoveryPhase::Failed(CeremonyFailure::Invalid).can_follow(None));
        assert!(RecoveryPhase::Prepared.can_follow(Some(&done())));
    }
}
