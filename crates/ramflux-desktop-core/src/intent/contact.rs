// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Actions on the contact link with a participant.

use serde::{Deserialize, Serialize};

use super::methods;
use crate::model::{ContactLink, Risk};

/// Longest contact id the desktop accepts, in bytes.
pub const MAX_CONTACT_ID_LEN: usize = 256;

/// Why a typed contact id cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactIdProblem {
    /// Nothing typed.
    Empty,
    /// Contains whitespace or control characters.
    Spaces,
    /// Longer than [`MAX_CONTACT_ID_LEN`].
    TooLong,
}

/// Checks a contact id typed by the user (already trimmed by the form).
///
/// # Errors
///
/// Returns the first problem found.
pub fn check_contact_id(id: &str) -> Result<(), ContactIdProblem> {
    if id.is_empty() {
        Err(ContactIdProblem::Empty)
    } else if id.chars().any(|c| c.is_whitespace() || c.is_control()) {
        Err(ContactIdProblem::Spaces)
    } else if id.len() > MAX_CONTACT_ID_LEN {
        Err(ContactIdProblem::TooLong)
    } else {
        Ok(())
    }
}

/// An action on an existing contact link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactAction {
    /// Accept an incoming request.
    Accept,
    /// Reject an incoming request.
    Reject,
    /// Block the participant (revokes what was shared with them).
    Block,
    /// Lift a block.
    Unblock,
    /// Remove the contact (revokes what was shared with them).
    Remove,
}

impl ContactAction {
    /// Every action.
    pub const ALL: [Self; 5] =
        [Self::Accept, Self::Reject, Self::Block, Self::Unblock, Self::Remove];

    /// The contract method.
    pub const fn method(self) -> &'static str {
        match self {
            Self::Accept => methods::CONTACT_ACCEPT,
            Self::Reject => methods::CONTACT_REJECT,
            Self::Block => methods::CONTACT_BLOCK,
            Self::Unblock => methods::CONTACT_UNBLOCK,
            Self::Remove => methods::CONTACT_REMOVE,
        }
    }

    /// Blocking and removing revoke object shares and capabilities: they
    /// need the attended human's hold.
    pub const fn risk(self) -> Risk {
        match self {
            Self::Block | Self::Remove => Risk::High,
            Self::Accept | Self::Reject | Self::Unblock => Risk::Low,
        }
    }

    /// Whether the action applies to a link in state `link`.
    pub const fn applies_to(self, link: ContactLink) -> bool {
        match self {
            Self::Accept | Self::Reject => matches!(link, ContactLink::Incoming),
            Self::Block => matches!(
                link,
                ContactLink::Accepted | ContactLink::Incoming | ContactLink::Outgoing
            ),
            Self::Unblock => matches!(link, ContactLink::Blocked),
            Self::Remove => matches!(link, ContactLink::Accepted | ContactLink::Blocked),
        }
    }

    /// The actions offered for a link in state `link`, in display order.
    pub fn offered(link: ContactLink) -> Vec<Self> {
        Self::ALL.into_iter().filter(|a| a.applies_to(link)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contact_ids_are_checked() {
        assert_eq!(check_contact_id(""), Err(ContactIdProblem::Empty));
        assert_eq!(check_contact_id("principal ana"), Err(ContactIdProblem::Spaces));
        assert_eq!(
            check_contact_id(&"a".repeat(MAX_CONTACT_ID_LEN + 1)),
            Err(ContactIdProblem::TooLong)
        );
        assert_eq!(check_contact_id("principal-ana"), Ok(()));
    }

    #[test]
    fn actions_follow_the_link_state() {
        assert_eq!(
            ContactAction::offered(ContactLink::Incoming),
            [ContactAction::Accept, ContactAction::Reject, ContactAction::Block]
        );
        assert_eq!(
            ContactAction::offered(ContactLink::Accepted),
            [ContactAction::Block, ContactAction::Remove]
        );
        assert_eq!(
            ContactAction::offered(ContactLink::Blocked),
            [ContactAction::Unblock, ContactAction::Remove]
        );
        assert!(ContactAction::offered(ContactLink::Removed).is_empty());
        assert!(ContactAction::offered(ContactLink::None).is_empty());
    }
}
