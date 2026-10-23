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
//! method that reaches a node (account creation, unlock, sends, pulls,
//! probes) waits for the network.

use std::time::Duration;

/// What a method does, for its budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    /// Reads local daemon state.
    Read,
    /// Changes local daemon state only.
    Local,
    /// Reaches a node or derives keys.
    Network,
    /// `daemon.stop`: drains and closes the daemon.
    Stop,
}

/// Methods that reach a node, or derive keys from a passphrase.
const NETWORK: [&str; 22] = [
    "account.create",
    "account.unlock",
    "account.backup.import",
    "account.backup.export",
    "account.passphrase.rotate",
    "node.add",
    "node.probe",
    "device.revoke",
    "message.receive",
    "group.send",
    "group.receive",
    "group.member.kick",
    "group.member.ban",
    "group.member.remove",
    "group.role.set",
    "group.member.add",
    "group.invite.create",
    "message.submit",
    "contact.verify",
    "object.get",
    "object.transfer.resume",
    "subscription.open",
];

/// Methods that only read.
const READS: [&str; 18] = [
    "daemon.status",
    "node.list",
    "account.list",
    "account.status",
    "device.list",
    "conversation.list",
    "group.list",
    "contact.list",
    "mcp.approval.list",
    "grant.list",
    "grant.list_standing_approvals",
    "message.list",
    "object.list",
    "mcp.tool.list",
    "mcp.server.list",
    "conversation.directory",
    "identity.directory",
    "object.directory",
];

/// The weight of `method`.
pub fn weight(method: &str) -> Weight {
    if method == "daemon.stop" {
        Weight::Stop
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
            read: Duration::from_secs(15),
            local: Duration::from_secs(20),
            network: Duration::from_secs(60),
            stop: Duration::from_secs(10),
        }
    }

    /// The same budget for every method (tests).
    pub const fn uniform(budget: Duration) -> Self {
        Self { read: budget, local: budget, network: budget, stop: budget }
    }

    /// The budget of `method`.
    pub fn budget(&self, method: &str) -> Duration {
        match weight(method) {
            Weight::Read => self.read,
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
        assert!(deadlines.read < deadlines.network);
    }

    #[test]
    fn a_deadline_is_absolute_and_in_the_future() {
        let now = deadline_ms(Duration::ZERO);
        let later = deadline_ms(Duration::from_secs(5));
        assert!(later >= now + 4_900 && later <= now + 5_100);
        assert!(now > 1_700_000_000_000, "milliseconds since the epoch");
    }
}
