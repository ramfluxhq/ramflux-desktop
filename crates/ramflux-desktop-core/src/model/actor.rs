// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Who proposes an intent and how a human confirmed it.

use serde::{Deserialize, Serialize};

use crate::ids::ParticipantId;

/// Who originates an intent or a task. Humans and agents share one pipeline (R6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Actor {
    /// The attended human user of this desktop.
    Human,
    /// An agent participant acting through the same policy as the human.
    Agent(ParticipantId),
    /// Another member of the conversation who asked for a task (a person
    /// or an agent; its kind is not verified). Never proposes intents here.
    Member(ParticipantId),
}

/// The gesture the attended human completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfirmGesture {
    /// The hold-to-confirm strip was held for its full duration.
    Hold,
}

/// Shortest hold the policy accepts as a confirmation, in milliseconds.
pub const MIN_HOLD_MS: u64 = 1_000;

/// Evidence that the attended human confirmed an intent: an unforgeable
/// capability.
///
/// It has no public constructor, is neither `Clone` nor `Deserialize`, and
/// can only be minted by a [`ConfirmationIssuer`] after a completed hold of
/// at least [`MIN_HOLD_MS`]. So agents never confirm: an agent proposal, a UI
/// event, a scenario file or a deserialized intent can never carry one, and
/// one confirmation cannot be reused for a second intent. It serializes so
/// the audit pair travels in the effect body.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct HumanConfirmation {
    gesture: ConfirmGesture,
    held_ms: u64,
}

impl HumanConfirmation {
    /// The completed gesture.
    pub const fn gesture(&self) -> ConfirmGesture {
        self.gesture
    }

    /// How long the control was held, in milliseconds.
    pub const fn held_ms(&self) -> u64 {
        self.held_ms
    }
}

/// The only source of [`HumanConfirmation`]s.
///
/// Deliberately not `Clone`: the application creates exactly one and hands
/// it to the hold-to-confirm strip's owner (the intent root), which mints a
/// confirmation only when the attended human completes a hold.
#[derive(Debug)]
pub struct ConfirmationIssuer {
    _private: (),
}

impl ConfirmationIssuer {
    /// Creates the issuer. Give it to the owner of the hold-to-confirm strip
    /// and to nothing else.
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self { _private: () }
    }

    /// Mints the confirmation for a completed hold of `held_ms`, or `None`
    /// when the hold was shorter than [`MIN_HOLD_MS`].
    #[must_use]
    pub const fn issue_hold(&self, held_ms: u64) -> Option<HumanConfirmation> {
        if held_ms < MIN_HOLD_MS {
            None
        } else {
            Some(HumanConfirmation { gesture: ConfirmGesture::Hold, held_ms })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_holds_mint_nothing() {
        let issuer = ConfirmationIssuer::new();
        assert!(issuer.issue_hold(MIN_HOLD_MS - 1).is_none());
        let confirmation = issuer.issue_hold(1_250).expect("a full hold");
        assert_eq!(confirmation.gesture(), ConfirmGesture::Hold);
        assert_eq!(confirmation.held_ms(), 1_250);
    }
}
