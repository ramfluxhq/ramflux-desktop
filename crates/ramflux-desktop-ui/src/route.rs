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
    /// The app lock holds an account closed: a lock page with the account
    /// code field and nothing else of the accounts.
    Lock,
    /// Nothing of the accounts is shown yet: an open account's app lock is
    /// not known (at start, or while the window comes back to the front).
    Cover,
}

/// Derives the page from the store.
///
/// A held account (its app lock closed, or its lock not yet known while the
/// daemon is read, `app_lock_reads`) covers the window: the lock page for a
/// closed one, a blank cover for an unknown one. Otherwise the workspace
/// opens when an account is unlocked, or when the source declares no account
/// at all but already serves contexts (a single implicit account). A locked
/// account always returns to entry. Without an account, entry stays on the
/// node step until a node is confirmed and no discovered node waits for a
/// decision.
///
/// The app locks are read from `locks` (on a live daemon, its store: it keeps
/// reporting a held account after the rendered session is gone); the rest of
/// the routing reads `store`.
pub fn page_for(store: &Store, locks: &Store, app_lock_reads: bool) -> Page {
    if locks.accounts().any(|a| locks.app_locked(&a.id)) {
        return Page::Lock;
    }
    if app_lock_reads
        && locks
            .accounts()
            .any(|a| a.lock_state == LockState::Unlocked && locks.app_lock(&a.id).is_none())
    {
        return Page::Cover;
    }
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
        assert_eq!(page_for(p.store(), p.store(), false), Page::Workspace);
        p.advance(5_000);
        assert_eq!(
            page_for(p.store(), p.store(), false),
            Page::Workspace,
            "a disconnect is shown, not routed"
        );
    }
}
