// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Effects: what a source must do as the result of an intent.

use serde_json::Value;

use super::{Intent, SecretInput};
use crate::model::AgentRuntimeAction;

/// What a source must do as a result of an intent.
#[derive(Debug)]
pub enum Effect {
    /// Perform a contract call.
    Call {
        /// One of [`super::methods`].
        method: String,
        /// Call arguments, including `proposed_by` and `confirmed_by`.
        body: Value,
    },
    /// Perform a contract call that carries one-shot secrets. The body never
    /// contains secret material; the secrets travel beside it and are dropped
    /// (zeroized) after use.
    SecretCall {
        /// One of [`super::methods`].
        method: String,
        /// Non-secret call arguments, including `proposed_by`.
        body: Value,
        /// Secrets in the order the method defines.
        secrets: Vec<SecretInput>,
    },
    /// Run a PRX runtime action on this computer, in process. Not a bus
    /// call; the source hands the key to PRX and keeps it nowhere else.
    AgentRuntime {
        /// The action.
        action: AgentRuntimeAction,
        /// The provider (install only).
        provider: String,
        /// The model (install only).
        model: String,
        /// The provider key (install only). Never printed or stored.
        key: Option<SecretInput>,
    },
    /// Policy requires explicit human confirmation before the intent may proceed.
    /// The app shows a confirm strip and resubmits the intent with
    /// `confirmed_by` set.
    RequireConfirmation {
        /// The intent awaiting confirmation.
        intent: Box<Intent>,
    },
}

impl Effect {
    /// Returns the method name of a call.
    pub fn method(&self) -> Option<&str> {
        match self {
            Self::Call { method, .. } | Self::SecretCall { method, .. } => Some(method),
            Self::RequireConfirmation { .. } | Self::AgentRuntime { .. } => None,
        }
    }

    /// Returns the (non-secret) body of a call.
    pub fn body(&self) -> Option<&Value> {
        match self {
            Self::Call { body, .. } | Self::SecretCall { body, .. } => Some(body),
            Self::RequireConfirmation { .. } | Self::AgentRuntime { .. } => None,
        }
    }
}
