// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Intents (what the user or an agent wants) and effects (what the source must do).

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use zeroize::Zeroize;

use crate::ids::{AccountId, ContextId, GrantId, MomentId, ParticipantId, SurfaceId, TaskId};
use crate::model::{Actor, ConflictResolution, Risk, Role};

/// A one-shot secret (passphrase, PIN) handed from the UI to the source.
///
/// It is deliberately not `Clone`, not `Serialize`, and its `Debug` output is
/// `<redacted>`, so it cannot leak through logs, scenario files, or the event
/// inspector. The source reads it with [`SecretInput::expose`]; the buffer is
/// zeroized when the handle is dropped.
pub struct SecretInput(String);

impl SecretInput {
    /// Wraps a secret.
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    /// Borrows the secret. Do not copy it into longer-lived storage.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether the secret is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Drop for SecretInput {
    fn drop(&mut self) {
        self.0.zeroize();
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

    /// Returns this intent as confirmed by the attended human.
    ///
    /// Used only by the hold-to-confirm strip after the gesture completes: the
    /// actor becomes [`Actor::Human`] (the human makes the decision, whoever
    /// proposed it) and `confirmed` is set on kinds that carry it. Other kinds
    /// are returned unchanged apart from the actor.
    #[must_use]
    pub fn into_human_confirmed(self) -> Self {
        let kind = match self.kind {
            IntentKind::Approve { grant_id, .. } => {
                IntentKind::Approve { grant_id, confirmed: true }
            }
            IntentKind::SurfaceAction { moment_id, surface_id, action_id, values, .. } => {
                IntentKind::SurfaceAction {
                    moment_id,
                    surface_id,
                    action_id,
                    values,
                    confirmed: true,
                }
            }
            IntentKind::Governance { context_id, op, .. } => {
                IntentKind::Governance { context_id, op, confirmed: true }
            }
            other => other,
        };
        Self { actor: Actor::Human, kind }
    }

    /// Whether this intent claims human confirmation.
    pub fn is_confirmed(&self) -> bool {
        matches!(
            self.kind,
            IntentKind::Approve { confirmed: true, .. }
                | IntentKind::SurfaceAction { confirmed: true, .. }
                | IntentKind::Governance { confirmed: true, .. }
        )
    }
}

/// A group or space governance operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GovernanceOp {
    /// Add a known participant as a member.
    AddMember {
        /// Participant to add.
        participant_id: ParticipantId,
        /// Initial role.
        role: Role,
    },
    /// Remove a member (for example on their request).
    RemoveMember {
        /// Member to remove.
        participant_id: ParticipantId,
    },
    /// Remove a member against their will.
    Kick {
        /// Member to remove.
        participant_id: ParticipantId,
    },
    /// Remove and block a participant from rejoining.
    Ban {
        /// Participant to ban.
        participant_id: ParticipantId,
    },
    /// Change a member's role.
    SetRole {
        /// Member.
        participant_id: ParticipantId,
        /// New role.
        role: Role,
    },
    /// Create an invitation link for a new member.
    CreateInvite {
        /// Role granted on joining.
        role: Role,
    },
}

impl GovernanceOp {
    /// Policy risk of the operation. `Kick`, `Ban` and `SetRole` are high risk
    /// and need human confirmation.
    pub const fn risk(&self) -> Risk {
        match self {
            Self::Kick { .. } | Self::Ban { .. } | Self::SetRole { .. } => Risk::High,
            Self::RemoveMember { .. } => Risk::Medium,
            Self::AddMember { .. } | Self::CreateInvite { .. } => Risk::Low,
        }
    }

    /// Contract method name.
    pub const fn method(&self) -> &'static str {
        match self {
            Self::AddMember { .. } => methods::GROUP_ADD_MEMBER,
            Self::RemoveMember { .. } => methods::GROUP_REMOVE_MEMBER,
            Self::Kick { .. } => methods::GROUP_KICK,
            Self::Ban { .. } => methods::GROUP_BAN,
            Self::SetRole { .. } => methods::GROUP_SET_ROLE,
            Self::CreateInvite { .. } => methods::GROUP_CREATE_INVITE,
        }
    }

    /// Participant the operation targets, if any.
    pub const fn target(&self) -> Option<&ParticipantId> {
        match self {
            Self::AddMember { participant_id, .. }
            | Self::RemoveMember { participant_id }
            | Self::Kick { participant_id }
            | Self::Ban { participant_id }
            | Self::SetRole { participant_id, .. } => Some(participant_id),
            Self::CreateInvite { .. } => None,
        }
    }

    /// Role carried by the operation, if any.
    pub const fn role(&self) -> Option<Role> {
        match self {
            Self::AddMember { role, .. }
            | Self::SetRole { role, .. }
            | Self::CreateInvite { role } => Some(*role),
            Self::RemoveMember { .. } | Self::Kick { .. } | Self::Ban { .. } => None,
        }
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
        /// The human completed the hold-to-confirm gesture (high-risk actions).
        #[serde(default)]
        confirmed: bool,
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
    /// Mark a participant's safety number as verified out of band. Human only.
    VerifySafetyNumber {
        /// Participant.
        participant_id: ParticipantId,
    },
    /// Change group or space membership or roles.
    Governance {
        /// Group or space.
        context_id: ContextId,
        /// Operation.
        op: GovernanceOp,
        /// The human completed the hold-to-confirm gesture (high-risk operations).
        #[serde(default)]
        confirmed: bool,
    },
    /// Continue the offered context on this device. Human only.
    AcceptHandoff {
        /// Handoff moment.
        moment_id: MomentId,
    },
    /// Decline a handoff offer. Human only.
    DeclineHandoff {
        /// Handoff moment.
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
    /// Verify a safety number.
    pub const SAFETY_VERIFY: &str = "safety.verify";
    /// Add a group member.
    pub const GROUP_ADD_MEMBER: &str = "group.add_member";
    /// Remove a group member.
    pub const GROUP_REMOVE_MEMBER: &str = "group.remove_member";
    /// Kick a group member.
    pub const GROUP_KICK: &str = "group.kick";
    /// Ban a participant from a group.
    pub const GROUP_BAN: &str = "group.ban";
    /// Change a member's role.
    pub const GROUP_SET_ROLE: &str = "group.set_role";
    /// Create a group invitation.
    pub const GROUP_CREATE_INVITE: &str = "group.create_invite";
    /// Accept a handoff.
    pub const HANDOFF_ACCEPT: &str = "handoff.accept";
    /// Decline a handoff.
    pub const HANDOFF_DECLINE: &str = "handoff.decline";
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
