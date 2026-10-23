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
}

/// The gesture the attended human completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfirmGesture {
    /// The hold-to-confirm strip was held for its full duration.
    Hold,
}

/// Evidence that the attended human confirmed an intent.
///
/// Only the hold-to-confirm strip creates it, after the gesture completes.
/// Agents never confirm: the application builds agent intents without it,
/// and a UI event can never carry it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanConfirmation {
    /// The completed gesture.
    pub gesture: ConfirmGesture,
    /// How long the control was held, in milliseconds.
    pub held_ms: u64,
}

impl HumanConfirmation {
    /// A completed hold of `held_ms` milliseconds.
    pub const fn hold(held_ms: u64) -> Self {
        Self { gesture: ConfirmGesture::Hold, held_ms }
    }
}
