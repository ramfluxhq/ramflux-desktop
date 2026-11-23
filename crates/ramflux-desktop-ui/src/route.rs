// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Which full-screen page the store calls for. Derived, never stored.

use ramflux_desktop_core::{LockState, SourceStatus, Store};

/// A step of the entry page (P0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryStep {
    /// The local daemon is not reachable yet.
    Daemon,
    /// No usable node yet, or a discovered node awaits confirmation.
    Node,
    /// Unlocking an existing account, or creating, joining or restoring one
    /// on a confirmed node.
    Account,
}

/// The page to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// P0 entry at a step.
    Entry(EntryStep),
    /// P1 workspace.
    Workspace,
}

/// Derives the page from the store.
///
/// The workspace opens when an account is unlocked, or when the source
/// declares no account at all but already serves contexts (a single implicit
/// account). A locked account always returns to entry. Without an account,
/// entry stays on the node step until a node is confirmed and no discovered
/// node waits for a decision.
pub fn page_for(store: &Store) -> Page {
    let mut accounts = store.accounts().peekable();
    if accounts.peek().is_none() {
        if store.contexts().next().is_some() {
            return Page::Workspace;
        }
    } else if store.accounts().any(|a| a.lock_state == LockState::Unlocked) {
        return Page::Workspace;
    }
    let pending_node = store.nodes().any(|n| !n.confirmed);
    let usable_node = store.nodes().any(|n| n.confirmed && n.enabled);
    let step = if store.accounts().next().is_some() {
        EntryStep::Account
    } else if store.source_status() == SourceStatus::Disconnected && store.nodes().next().is_none()
    {
        EntryStep::Daemon
    } else if pending_node || !usable_node {
        EntryStep::Node
    } else {
        EntryStep::Account
    };
    Page::Entry(step)
}

#[cfg(test)]
mod tests {
    use ramflux_desktop_test::{ScenarioPlayer, load_fixture};

    use super::*;

    #[test]
    fn implicit_account_scenarios_open_the_workspace_and_stay_there() {
        let mut p = ScenarioPlayer::new(&load_fixture("s08_failures").expect("s08"));
        p.advance(100);
        assert_eq!(page_for(p.store()), Page::Workspace);
        p.advance(5_000);
        assert_eq!(page_for(p.store()), Page::Workspace, "a disconnect is shown, not routed");
    }
}
