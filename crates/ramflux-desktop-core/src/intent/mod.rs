// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Intents (what the user or an agent wants) and effects (what the source must do).
//!
//! - [`Intent`]: who proposed it, who confirmed it, and what is wanted.
//! - [`IntentKind`]: every intent, grouped by area.
//! - [`GovernanceOp`]: group and space governance operations.
//! - [`ContactAction`]: actions on a contact link, and the contact id check.
//! - [`Effect`]: what the source must execute.
//! - [`methods`]: contract method names.
//! - [`SecretInput`]: one-shot secrets that never serialize.

mod contact;
mod effect;
mod governance;
mod kind;
pub mod methods;
mod secret;

use serde::{Deserialize, Serialize};

use crate::model::{Actor, HumanConfirmation};

pub use contact::{ContactAction, ContactIdProblem, MAX_CONTACT_ID_LEN, check_contact_id};
pub use effect::Effect;
pub use governance::GovernanceOp;
pub use kind::{IntentKind, SyncDirection};
pub use secret::SecretInput;

/// An intent: a request to change business state.
///
/// View-local behavior (expand, hover, scroll) is never an intent. The audit
/// record of an intent is the pair `proposed_by` and `confirmed_by`: an agent
/// may propose, only the attended human confirms, and both travel in the
/// effect body.
#[derive(Debug, Serialize, Deserialize)]
pub struct Intent {
    /// Who proposed it. Never rewritten by a confirmation.
    pub proposed_by: Actor,
    /// Set only by the hold-to-confirm strip after the gesture completes.
    /// Never deserialized: a stored or received intent is always unconfirmed.
    #[serde(default, skip_deserializing)]
    pub confirmed_by: Option<HumanConfirmation>,
    /// What is wanted.
    pub kind: IntentKind,
}

impl Intent {
    /// An unconfirmed intent proposed by the attended human.
    pub fn human(kind: IntentKind) -> Self {
        Self { proposed_by: Actor::Human, confirmed_by: None, kind }
    }

    /// An intent proposed by an agent participant. It never carries a
    /// confirmation; only a later hold by the human can add one.
    pub fn agent(agent: crate::ids::ParticipantId, kind: IntentKind) -> Self {
        Self { proposed_by: Actor::Agent(agent), confirmed_by: None, kind }
    }

    /// Returns this intent with the attended human's confirmation attached.
    ///
    /// Used only by the hold-to-confirm strip after the gesture completes.
    /// `proposed_by` is kept, so the audit shows who proposed and who confirmed.
    #[must_use]
    pub fn confirmed_by_human(self, confirmation: HumanConfirmation) -> Self {
        Self { confirmed_by: Some(confirmation), ..self }
    }

    /// Whether the attended human confirmed this intent.
    pub fn is_confirmed(&self) -> bool {
        self.confirmed_by.is_some()
    }
}
