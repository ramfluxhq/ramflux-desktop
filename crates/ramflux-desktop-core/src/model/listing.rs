// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Whether a collection has been listed by the source yet.
//!
//! An empty collection means two different things: the source has not
//! answered yet (loading), or it answered and there is nothing (empty). The
//! source declares each listing once with `DomainEvent::ListingsLoaded`; the
//! UI then shows an explicit empty state instead of waiting forever.

use serde::{Deserialize, Serialize};

/// A collection the source lists in full.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Listing {
    /// Node profiles.
    Nodes,
    /// Local accounts.
    Accounts,
    /// Conversation contexts.
    Contexts,
    /// Contacts.
    Contacts,
    /// Objects and transfers.
    Objects,
    /// Approvals (grants).
    Approvals,
}

impl Listing {
    /// Every listing.
    pub const ALL: [Self; 6] = [
        Self::Nodes,
        Self::Accounts,
        Self::Contexts,
        Self::Contacts,
        Self::Objects,
        Self::Approvals,
    ];
}

/// What a list view shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListState {
    /// The source has not listed the collection yet.
    Loading,
    /// Listed, and nothing is in it.
    Empty,
    /// There is something to show.
    Ready,
}

impl ListState {
    /// The state of a list with `empty` contents whose collection is
    /// `listed` or not. Contents always win: what is there is shown.
    pub const fn of(listed: bool, empty: bool) -> Self {
        match (empty, listed) {
            (false, _) => Self::Ready,
            (true, true) => Self::Empty,
            (true, false) => Self::Loading,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contents_win_then_listing_decides() {
        assert_eq!(ListState::of(false, false), ListState::Ready);
        assert_eq!(ListState::of(true, false), ListState::Ready);
        assert_eq!(ListState::of(true, true), ListState::Empty);
        assert_eq!(ListState::of(false, true), ListState::Loading);
    }
}
