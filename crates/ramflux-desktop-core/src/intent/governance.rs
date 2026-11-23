// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Group and space governance operations.

use serde::{Deserialize, Serialize};

use super::methods;
use crate::ids::ParticipantId;
use crate::model::{Risk, Role};

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
    /// Change a member's role.
    SetRole {
        /// Member.
        participant_id: ParticipantId,
        /// New role.
        role: Role,
    },
}

impl GovernanceOp {
    /// Policy risk of the operation. `Kick` and `SetRole` are high risk
    /// and need human confirmation.
    pub const fn risk(&self) -> Risk {
        match self {
            Self::Kick { .. } | Self::SetRole { .. } => Risk::High,
            Self::RemoveMember { .. } => Risk::Medium,
            Self::AddMember { .. } => Risk::Low,
        }
    }

    /// Contract method name.
    pub const fn method(&self) -> &'static str {
        match self {
            Self::AddMember { .. } => methods::GROUP_ADD_MEMBER,
            Self::RemoveMember { .. } => methods::GROUP_REMOVE_MEMBER,
            Self::Kick { .. } => methods::GROUP_KICK,
            Self::SetRole { .. } => methods::GROUP_SET_ROLE,
        }
    }

    /// Participant the operation targets, if any.
    pub const fn target(&self) -> Option<&ParticipantId> {
        match self {
            Self::AddMember { participant_id, .. }
            | Self::RemoveMember { participant_id }
            | Self::Kick { participant_id }
            | Self::SetRole { participant_id, .. } => Some(participant_id),
        }
    }

    /// Role carried by the operation, if any.
    pub const fn role(&self) -> Option<Role> {
        match self {
            Self::AddMember { role, .. } | Self::SetRole { role, .. } => Some(*role),
            Self::RemoveMember { .. } | Self::Kick { .. } => None,
        }
    }
}
