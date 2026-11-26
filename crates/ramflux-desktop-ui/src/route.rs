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

    #[test]
    fn a_lock_the_daemon_starts_shows_the_lock_page_without_a_focus_change() {
        use ramflux_desktop_core::{
            Account, AccountId, AccountKind, AppLock, AppLockState, DomainEvent, EventEnvelope,
            LockState, Networking, SecureSessions, Timestamp,
        };

        let id = AccountId::new("a-1");
        let account = Account {
            id: id.clone(),
            label: "a-1".to_owned(),
            kind: AccountKind::HumanInteractive,
            node_id: None,
            device_id: None,
            lock_state: LockState::Unlocked,
            self_participant_id: None,
            secure_sessions: SecureSessions::Ready,
            networking_session: None,
            networking_custody: None,
            authority_blocked: false,
            networking_failure: None,
            networking_reason: None,
            networking: Networking::Live,
        };
        let lock = |state, seq| {
            EventEnvelope::new(
                seq,
                Timestamp(seq as i64),
                DomainEvent::AppLockChanged {
                    account_id: id.clone(),
                    lock: AppLock {
                        enabled: true,
                        timeout_ms: 1_000,
                        state,
                        reason: None,
                        generation: 1,
                    },
                },
            )
        };
        let opened = EventEnvelope::new(1, Timestamp(1), DomainEvent::AccountUpdated(account));
        // The rendered account's own session never hears of the daemon's
        // timer: the page is routed from the daemon session's store.
        let mut session = Store::new();
        session.apply(&opened).expect("session account");
        let mut daemon = Store::new();
        daemon.apply(&opened).expect("daemon account");
        daemon.apply(&lock(AppLockState::Unlocked, 2)).expect("unlocked");
        assert_eq!(page_for(&session, &daemon, true), Page::Workspace);
        // The daemon's timer closed the lock while the window stayed unfocused:
        // the next read of its state is all the daemon's store gets.
        daemon.apply(&lock(AppLockState::Locked, 3)).expect("locked");
        assert_eq!(page_for(&session, &daemon, true), Page::Lock);
        assert!(session.app_lock(&id).is_none(), "the session store is not told");
    }
}
