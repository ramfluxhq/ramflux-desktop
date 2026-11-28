// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Per-request deadlines.
//!
//! Every frame carries `deadline_ms`, an absolute Unix time in
//! milliseconds. The daemon refuses a request whose deadline passed before
//! it was applied, and the client stops waiting at the same instant, so no
//! request can leave the desktop waiting forever: it answers, fails, or
//! times out (`BusError::Timeout`, shown as a timeout).
//!
//! The budget depends on what the method does: a read of local state is
//! quick, a local change takes a write and possibly key derivation, and a
//! method that reaches a node (account creation, sends, pulls, probes)
//! waits for the network. Local unlock has its own key-derivation budget.

use std::time::Duration;

/// What a method does, for its budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    /// Reads local daemon state.
    Read,
    /// Unlocks local account state, including key derivation.
    Unlock,
    /// Changes the app lock, which may wait for the account code: the unlock
    /// budget plus the window the code sheet has. Its budget is 90 s, as on
    /// the phones.
    Attended,
    /// Locks or switches the active account.
    Session,
    /// Changes local daemon state only.
    Local,
    /// Reaches a node or derives keys.
    Network,
    /// `daemon.stop`: drains and closes the daemon.
    Stop,
}

/// Methods that reach a node, or derive keys from a passphrase.
const NETWORK: [&str; 37] = [
    "account.create",
    "account.backup.create",
    "account.restore.verify",
    // Answers once the job's worker stopped (a file fetch may be under way).
    "account.backup.cancel",
    // The daemon stages the agent and publishes its runtime settings.
    "agent.create",
    "account.passphrase.rotate",
    "node.add",
    "node.probe",
    "device.revoke",
    "message.receive",
    "group.send",
    "group.name.set",
    "group.member.kick",
    "group.member.remove",
    "group.role.set",
    "group.member.add",
    "message.submit",
    "object.get",
    // A card image fetch (one attempt may import for up to a minute).
    "object.fetch",
    "object.transfer.resume",
    "subscription.open",
    "call.invite",
    "call.answer",
    "call.reject",
    "call.hangup",
    "call.media.session",
    "conversation.reply",
    "conversation.edit",
    "conversation.delete",
    "message.receipt.read",
    "group.leave",
    "group.invitation.accept",
    "group.invitation.decline",
    "username.status",
    "username.contact.add",
    "username.mutate",
    "username.resolve",
];

/// Methods that only read.
const READS: [&str; 22] = [
    "daemon.status",
    "node.list",
    "account.list",
    "account.status",
    "account.app_lock.state",
    "device.list",
    "conversation.list",
    "group.list",
    "contact.list",
    "mcp.approval.list",
    "grant.list",
    "grant.list_standing_approvals",
    "message.read",
    "object.list",
    "mcp.tool.list",
    "mcp.server.list",
    "conversation.directory",
    "identity.directory",
    "object.directory",
    "account.backup.jobs",
    "contact.verification.status",
    "conversation.security",
];

/// The weight of `method`.
pub fn weight(method: &str) -> Weight {
    if method == "daemon.stop" {
        Weight::Stop
    } else if method == "account.unlock" {
        Weight::Unlock
    } else if method == "account.app_lock.configure" {
        Weight::Attended
    } else if matches!(method, "account.lock" | "account.switch") {
        Weight::Session
    } else if NETWORK.contains(&method) {
        Weight::Network
    } else if READS.contains(&method) {
        Weight::Read
    } else {
        Weight::Local
    }
}

/// Budgets by weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deadlines {
    /// Reads of local state.
    pub read: Duration,
    /// Local account unlock.
    pub unlock: Duration,
    /// An app lock change that may wait for the account code.
    pub attended: Duration,
    /// Account lock and switch.
    pub session: Duration,
    /// Local changes.
    pub local: Duration,
    /// Node round trips and key derivation.
    pub network: Duration,
    /// A graceful daemon stop.
    pub stop: Duration,
}

impl Deadlines {
    /// The application's budgets.
    pub const fn standard() -> Self {
        Self {
            read: Duration::from_secs(2),
            unlock: Duration::from_secs(10),
            attended: Duration::from_secs(90),
            session: Duration::from_secs(5),
            local: Duration::from_secs(20),
            network: Duration::from_secs(60),
            stop: Duration::from_secs(10),
        }
    }

    /// The same budget for every method (tests).
    pub const fn uniform(budget: Duration) -> Self {
        Self {
            read: budget,
            unlock: budget,
            attended: budget,
            session: budget,
            local: budget,
            network: budget,
            stop: budget,
        }
    }

    /// The budget of `method`.
    pub fn budget(&self, method: &str) -> Duration {
        match weight(method) {
            Weight::Read => self.read,
            Weight::Unlock => self.unlock,
            Weight::Attended => self.attended,
            Weight::Session => self.session,
            Weight::Local => self.local,
            Weight::Network => self.network,
            Weight::Stop => self.stop,
        }
    }
}

impl Default for Deadlines {
    fn default() -> Self {
        Self::standard()
    }
}

/// The absolute Unix deadline, in milliseconds, `budget` from now.
pub fn deadline_ms(budget: Duration) -> u64 {
    let now =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    u64::try_from((now + budget).as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn methods_get_the_budget_of_what_they_do() {
        let deadlines = Deadlines::standard();
        assert_eq!(deadlines.budget("daemon.status"), deadlines.read);
        assert_eq!(deadlines.budget("contact.block"), deadlines.local);
        assert_eq!(deadlines.budget("group.send"), deadlines.network);
        assert_eq!(deadlines.budget("daemon.stop"), deadlines.stop);
        // Creating an agent stages it and publishes it, like a network call.
        assert_eq!(deadlines.budget("agent.create"), deadlines.network);
        assert!(deadlines.read < deadlines.network);
    }

    #[test]
    fn unlock_budget_and_weights() {
        let deadlines = Deadlines::standard();
        assert_eq!(weight("account.unlock"), Weight::Unlock);
        assert!(!NETWORK.contains(&"account.unlock"));
        for (method, seconds) in [
            ("account.unlock", 10),
            ("account.lock", 5),
            ("account.switch", 5),
            ("account.status", 2),
            ("contact.list", 2),
            ("contact.block", 20),
            ("message.submit", 60),
            ("daemon.stop", 10),
        ] {
            assert_eq!(deadlines.budget(method), Duration::from_secs(seconds), "{method}");
            assert_eq!(
                Deadlines::uniform(Duration::from_millis(7)).budget(method),
                Duration::from_millis(7)
            );
        }
        assert_eq!(crate::config::SdkConfig::default().connect_timeout, Duration::from_secs(2));
    }

    #[test]
    fn a_deadline_is_absolute_and_in_the_future() {
        let now = deadline_ms(Duration::ZERO);
        let later = deadline_ms(Duration::from_secs(5));
        assert!(later >= now + 4_900 && later <= now + 5_100);
        assert!(now > 1_700_000_000_000, "milliseconds since the epoch");
    }
}
