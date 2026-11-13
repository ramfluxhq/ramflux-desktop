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
    /// The host-drawn Confirm button was pressed once (a click, a tap, or
    /// Enter or Space on the focused button).
    Tap,
}

/// Evidence that the attended human confirmed an intent: an unforgeable
/// capability.
///
/// It has no public constructor, is neither `Clone` nor `Deserialize`, and
/// can only be minted by a [`ConfirmationIssuer`] when the human presses the
/// host-drawn Confirm button. So agents never confirm: an agent proposal, a
/// UI event, a scenario file or a deserialized intent can never carry one,
/// and one confirmation cannot be reused for a second intent. It serializes
/// so the audit pair travels in the effect body.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct HumanConfirmation {
    gesture: ConfirmGesture,
}

impl HumanConfirmation {
    /// The completed gesture.
    pub const fn gesture(&self) -> ConfirmGesture {
        self.gesture
    }
}

/// The only source of [`HumanConfirmation`]s.
///
/// Deliberately not `Clone`: the application creates exactly one and hands
/// it to the confirm strip's owner (the intent root), which mints a
/// confirmation only when the attended human presses Confirm.
#[derive(Debug)]
pub struct ConfirmationIssuer {
    _private: (),
}

impl ConfirmationIssuer {
    /// Creates the issuer. Give it to the owner of the confirm strip and to
    /// nothing else.
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self { _private: () }
    }

    /// Mints the confirmation for one press of the host-drawn Confirm button.
    #[must_use]
    pub const fn issue_tap(&self) -> HumanConfirmation {
        HumanConfirmation { gesture: ConfirmGesture::Tap }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tap_mints_a_tap_confirmation() {
        let confirmation = ConfirmationIssuer::new().issue_tap();
        assert_eq!(confirmation.gesture(), ConfirmGesture::Tap, "test: the gesture is recorded");
    }
}
