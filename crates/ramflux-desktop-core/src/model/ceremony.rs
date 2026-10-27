// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The creation ceremony of a born-v3 account (DESK-V3-01).
//!
//! Only what is safe to keep lives here: the phase, the ceremony id, the
//! expiry and, for the confirmation, which three word positions were asked.
//! The recovery phrase itself never enters the store: it reaches the screen
//! through [`crate::Source::take_recovery_phrase`] and is dropped there when
//! the user hides it. The words the user types back are
//! [`crate::SecretInput`]s.
//!
//! ```text
//! (none | ended) -> Prepared -> PhraseReady -> Challenged -> Created
//! Prepared | PhraseReady | Challenged -> Failed | Cancelled
//! (none | ended) -> Failed            (the daemon refused to begin)
//! ```
//!
//! `Created`, `Failed` and `Cancelled` end the ceremony: a new
//! one may begin. The ceremony allows one confirmation attempt; a wrong
//! word ends it with [`CeremonyFailure::ConfirmationFailed`] and nothing is
//! created.

use serde::{Deserialize, Serialize};

use crate::ids::Timestamp;

/// How long a ceremony lives after the daemon prepared it, in milliseconds
/// (five minutes of the daemon's monotonic time).
pub const CEREMONY_TTL_MS: i64 = 300_000;

/// How many words the user types back.
pub const CHALLENGE_WORDS: usize = 3;

/// Why a ceremony ended without an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CeremonyFailure {
    /// The daemon does not offer born-v3 creation (`CustodyCreationDisabled`).
    Disabled,
    /// The words did not match; the only attempt is spent
    /// (`CustodyConfirmationFailed`).
    ConfirmationFailed,
    /// The device has no usable primary vault (`CustodyVaultUnavailable`).
    VaultUnavailable,
    /// The five minutes passed before the ceremony was confirmed.
    Expired,
    /// The daemon refused a step as out of order, replayed or not valid
    /// (`CustodySetupInvalid`), or could not be reached.
    Invalid,
}

/// Whether the new account is connected to its node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Networking {
    /// Registered and connected.
    Live,
    /// Exists on this device only: registration comes later.
    Pending,
}

/// The account that was created (the ordinary account-create answer), as
/// far as the desktop may show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CeremonyAccount {
    /// The id the daemon keeps the account under on this device.
    pub local_account_id: String,
    /// The name the user gave it.
    pub display_name: String,
    /// Whether it is connected.
    pub networking: Networking,
}

/// Where a ceremony is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CeremonyPhase {
    /// The daemon prepared the recovery material; nothing was shown.
    Prepared,
    /// The daemon handed the phrase out once: it waits in
    /// [`crate::Source::take_recovery_phrase`] (or is on the screen).
    PhraseReady,
    /// The screen erased the phrase and the daemon chose the words to ask,
    /// one-based positions in the phrase, all different.
    Challenged {
        /// The three positions, in the order they are asked.
        positions: [u8; CHALLENGE_WORDS],
    },
    /// The account exists on this device and is unlocked; its networking is
    /// live or still pending (registration comes later).
    Created(CeremonyAccount),
    /// The ceremony ended without an account.
    Failed(CeremonyFailure),
    /// The user cancelled.
    Cancelled,
}

impl CeremonyPhase {
    /// Whether the ceremony is over (a new one may begin).
    pub fn is_ended(&self) -> bool {
        matches!(self, Self::Created(_) | Self::Failed(_) | Self::Cancelled)
    }

    /// Whether a ceremony in `from` (none when there is no ceremony) may
    /// move to `self`.
    pub fn can_follow(&self, from: Option<&Self>) -> bool {
        let Some(from) = from.filter(|from| !from.is_ended()) else {
            return matches!(self, Self::Prepared | Self::Failed(_));
        };
        matches!(
            (from, self),
            (Self::Prepared, Self::PhraseReady)
                | (Self::PhraseReady, Self::Challenged { .. })
                | (Self::Challenged { .. }, Self::Created(_))
                | (_, Self::Failed(_) | Self::Cancelled)
        )
    }
}

/// A creation ceremony as the daemon reports it (the whole record).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ceremony {
    /// Where it is.
    pub phase: CeremonyPhase,
    /// The commitment to the new account's root, public; empty when the
    /// daemon refused to begin. Stays the same for the whole ceremony.
    pub ceremony_id: String,
    /// The name the user asked for.
    pub label: String,
    /// When the ceremony expires. Stays the same for the whole ceremony.
    pub expires_at: Timestamp,
}

impl Ceremony {
    /// Whether the ceremony is still running.
    pub fn is_active(&self) -> bool {
        !self.phase.is_ended()
    }

    /// Whether the five minutes are over at `now`.
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

    fn challenged() -> CeremonyPhase {
        CeremonyPhase::Challenged { positions: [3, 11, 20] }
    }

    fn account() -> CeremonyPhase {
        CeremonyPhase::Created(CeremonyAccount {
            local_account_id: "a".into(),
            display_name: "A".into(),
            networking: Networking::Pending,
        })
    }

    #[test]
    fn the_ceremony_walks_forward_one_phase_at_a_time() {
        let prepared = CeremonyPhase::Prepared;
        let ready = CeremonyPhase::PhraseReady;
        assert!(prepared.can_follow(None));
        assert!(ready.can_follow(Some(&prepared)));
        assert!(challenged().can_follow(Some(&ready)));
        assert!(account().can_follow(Some(&challenged())));
        // No skipping, no going back.
        assert!(!ready.can_follow(None));
        assert!(!challenged().can_follow(Some(&prepared)));
        assert!(!account().can_follow(Some(&ready)));
        assert!(!prepared.can_follow(Some(&ready)));
        assert!(!ready.can_follow(Some(&ready)));
    }

    #[test]
    fn a_running_ceremony_can_fail_or_be_cancelled_from_any_phase() {
        for from in [CeremonyPhase::Prepared, CeremonyPhase::PhraseReady, challenged()] {
            assert!(CeremonyPhase::Cancelled.can_follow(Some(&from)));
            assert!(CeremonyPhase::Failed(CeremonyFailure::Expired).can_follow(Some(&from)));
        }
    }

    #[test]
    fn an_ended_ceremony_only_makes_room_for_a_new_one() {
        for ended in [
            account(),
            CeremonyPhase::Cancelled,
            CeremonyPhase::Failed(CeremonyFailure::ConfirmationFailed),
        ] {
            assert!(ended.is_ended());
            assert!(CeremonyPhase::Prepared.can_follow(Some(&ended)));
            assert!(!CeremonyPhase::PhraseReady.can_follow(Some(&ended)));
        }
        // A refused begin is a failure with no ceremony before it.
        assert!(CeremonyPhase::Failed(CeremonyFailure::Disabled).can_follow(None));
        assert!(!CeremonyPhase::Cancelled.can_follow(None));
    }

    #[test]
    fn the_expiry_is_read_against_the_given_time() {
        let ceremony = Ceremony {
            phase: CeremonyPhase::Prepared,
            ceremony_id: "x".into(),
            label: "L".into(),
            expires_at: Timestamp(1_000),
        };
        assert!(!ceremony.expired_at(Timestamp(999)));
        assert!(ceremony.expired_at(Timestamp(1_000)));
        assert_eq!(ceremony.remaining_ms(Timestamp(400)), 600);
        assert_eq!(ceremony.remaining_ms(Timestamp(5_000)), 0);
    }
}
