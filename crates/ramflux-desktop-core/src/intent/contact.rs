// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Adding a contact (by the principal id or the contact card they shared)
//! and actions on the contact link with a participant.

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

/// Longest contact card the desktop accepts, in bytes (a card is a short
/// signed record; anything longer is not one).
pub const MAX_CONTACT_CARD_LEN: usize = 16 * 1024;

/// Why pasted or imported contact card text cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactCardProblem {
    /// Nothing pasted.
    Empty,
    /// Longer than [`MAX_CONTACT_CARD_LEN`].
    TooLong,
    /// Contains control characters other than line breaks and tabs.
    Control,
}

/// What the user typed or pasted into the add-contact field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactInput<'a> {
    /// A bare principal id: the daemon looks it up on the contact's home
    /// node (trusted on first use until safety numbers are compared).
    PrincipalId(&'a str),
    /// A contact card (text or link): the daemon checks its signature and
    /// pins the identity it names.
    Card(&'a str),
}

/// Tells a principal id from a contact card. A card is shared as a link
/// (it names a scheme, so it holds a `:`), as several lines, or as one long
/// encoded line (longer than any id, [`MAX_CONTACT_ID_LEN`]); anything else
/// is a principal id, which [`check_contact_id`] then checks. The daemon
/// decides whether a card is valid. The input is trimmed.
pub fn classify_contact_input(input: &str) -> ContactInput<'_> {
    let input = input.trim();
    let card =
        input.contains(':') || input.contains(['\n', '\r']) || input.len() > MAX_CONTACT_ID_LEN;
    if card { ContactInput::Card(input) } else { ContactInput::PrincipalId(input) }
}

/// Checks contact card text before it is sent to the daemon (already
/// trimmed by the form).
///
/// # Errors
///
/// Returns the first problem found.
pub fn check_contact_card(card: &str) -> Result<(), ContactCardProblem> {
    if card.is_empty() {
        Err(ContactCardProblem::Empty)
    } else if card.len() > MAX_CONTACT_CARD_LEN {
        Err(ContactCardProblem::TooLong)
    } else if card.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
        Err(ContactCardProblem::Control)
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
    fn principal_ids_and_cards_are_told_apart() {
        assert_eq!(
            classify_contact_input("  principal-93d0  "),
            ContactInput::PrincipalId("principal-93d0")
        );
        assert_eq!(classify_contact_input(""), ContactInput::PrincipalId(""));
        assert_eq!(
            classify_contact_input("ramflux:contact/abc"),
            ContactInput::Card("ramflux:contact/abc")
        );
        assert_eq!(
            classify_contact_input("line one\nline two"),
            ContactInput::Card("line one\nline two")
        );
        let long = "a".repeat(MAX_CONTACT_ID_LEN + 1);
        assert_eq!(classify_contact_input(&long), ContactInput::Card(long.as_str()));
    }

    #[test]
    fn cards_are_checked() {
        assert_eq!(check_contact_card(""), Err(ContactCardProblem::Empty));
        assert_eq!(check_contact_card("card\u{7}"), Err(ContactCardProblem::Control));
        assert_eq!(
            check_contact_card(&"a".repeat(MAX_CONTACT_CARD_LEN + 1)),
            Err(ContactCardProblem::TooLong)
        );
        assert_eq!(check_contact_card("ramflux:contact/abc\nsig"), Ok(()));
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
