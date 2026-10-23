// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Intents (what the user or an agent wants) and effects (what the source must do).

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{AccountId, ContextId, GrantId, MomentId, SurfaceId, TaskId};
use crate::model::{Actor, ConflictResolution};

/// A one-shot secret (passphrase, PIN) handed from the UI to the source.
///
/// It is deliberately not `Clone`, not `Serialize`, and its `Debug` output is
/// `<redacted>`, so it cannot leak through logs, scenario files, or the event
/// inspector. The source consumes it with [`SecretInput::expose`].
pub struct SecretInput(String);

impl SecretInput {
    /// Wraps a secret.
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    /// Consumes the handle and returns the secret.
    pub fn expose(self) -> String {
        self.0
    }
}

impl fmt::Debug for SecretInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// An intent: a request to change business state.
///
/// View-local behavior (expand, hover, scroll) is never an intent.
#[derive(Debug, Serialize, Deserialize)]
pub struct Intent {
    /// Who wants it.
    pub actor: Actor,
    /// What is wanted.
    pub kind: IntentKind,
}

impl Intent {
    /// Builds a human intent.
    pub fn human(kind: IntentKind) -> Self {
        Self { actor: Actor::Human, kind }
    }
}

/// Every intent kind.
///
/// `Unlock` carries a [`SecretInput`] and is excluded from serde: serializing
/// it is an error and it cannot be deserialized.
#[derive(Debug, Serialize, Deserialize)]
pub enum IntentKind {
    /// Send a text message.
    SendMessage {
        /// Target context.
        context_id: ContextId,
        /// Text.
        text: String,
    },
    /// Send a local file as an object.
    AttachObject {
        /// Target context.
        context_id: ContextId,
        /// Local path chosen by the user.
        path: String,
    },
    /// Invoke an action of an agent surface.
    SurfaceAction {
        /// Moment holding the surface.
        moment_id: MomentId,
        /// Surface.
        surface_id: SurfaceId,
        /// Action id.
        action_id: String,
        /// Form values collected by the surface.
        #[serde(default)]
        values: Value,
    },
    /// Approve a pending grant.
    Approve {
        /// Grant.
        grant_id: GrantId,
        /// The human completed the hold-to-confirm gesture.
        #[serde(default)]
        confirmed: bool,
    },
    /// Deny a pending grant.
    Deny {
        /// Grant.
        grant_id: GrantId,
    },
    /// Reveal masked content (audited).
    Reveal {
        /// PrivacyReveal moment.
        moment_id: MomentId,
    },
    /// Resolve a CAS conflict.
    ResolveConflict {
        /// Conflict moment.
        moment_id: MomentId,
        /// Chosen resolution.
        resolution: ConflictResolution,
    },
    /// Start a task from a one-sentence request.
    StartTask {
        /// Context.
        context_id: ContextId,
        /// The request text.
        request: String,
    },
    /// Cancel a running task.
    CancelTask {
        /// Task.
        task_id: TaskId,
    },
    /// Pin a context to the stack.
    PinContext {
        /// Context.
        context_id: ContextId,
    },
    /// Unpin a context.
    UnpinContext {
        /// Context.
        context_id: ContextId,
    },
    /// Accept a contact request.
    AcceptContact {
        /// ContactRequest moment.
        moment_id: MomentId,
    },
    /// Reject a contact request.
    RejectContact {
        /// ContactRequest moment.
        moment_id: MomentId,
    },
    /// Unlock an account. Not serializable.
    #[serde(skip)]
    Unlock {
        /// Account.
        account_id: AccountId,
        /// One-shot secret.
        secret: SecretInput,
    },
}

/// Desktop contract method names used in [`Effect::Call`].
///
/// These are contract names, not daemon RPC names; an SDK-backed source maps
/// them to daemon calls.
pub mod methods {
    /// Send a text message.
    pub const SEND_MESSAGE: &str = "message.send";
    /// Send an object.
    pub const ATTACH_OBJECT: &str = "object.send";
    /// Invoke a surface action.
    pub const SURFACE_ACTION: &str = "surface.action";
    /// Approve a grant.
    pub const GRANT_APPROVE: &str = "grant.approve";
    /// Deny a grant.
    pub const GRANT_DENY: &str = "grant.deny";
    /// Reveal masked content.
    pub const PRIVACY_REVEAL: &str = "privacy.reveal";
    /// Resolve a conflict.
    pub const CONFLICT_RESOLVE: &str = "object.resolve_conflict";
    /// Start a task.
    pub const TASK_START: &str = "task.start";
    /// Cancel a task.
    pub const TASK_CANCEL: &str = "task.cancel";
    /// Pin a context.
    pub const CONTEXT_PIN: &str = "context.pin";
    /// Unpin a context.
    pub const CONTEXT_UNPIN: &str = "context.unpin";
    /// Accept a contact request.
    pub const CONTACT_ACCEPT: &str = "contact.accept";
    /// Reject a contact request.
    pub const CONTACT_REJECT: &str = "contact.reject";
}

/// What a source must do as a result of an intent.
#[derive(Debug)]
pub enum Effect {
    /// Perform a contract call.
    Call {
        /// One of [`methods`].
        method: String,
        /// Call arguments, including the acting `actor`.
        body: Value,
    },
    /// Policy requires explicit human confirmation before the intent may proceed.
    /// The app shows a hold-to-confirm bar and resubmits a confirmed human intent.
    RequireConfirmation {
        /// The intent awaiting confirmation.
        intent: Box<Intent>,
    },
    /// Unlock an account with a one-shot secret.
    Unlock {
        /// Account.
        account_id: AccountId,
        /// One-shot secret.
        secret: SecretInput,
    },
}

impl Effect {
    /// Returns the method name of a `Call`.
    pub fn method(&self) -> Option<&str> {
        match self {
            Self::Call { method, .. } => Some(method),
            Self::RequireConfirmation { .. } | Self::Unlock { .. } => None,
        }
    }
}
