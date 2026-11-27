// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The agent roster as the screens show it: one group per named agent, and
//! the typed reasons for what this build does not offer. Pure: it reads the
//! store and returns borrowed rows.

use crate::model::{AgentRecord, AgentUnavailable};
use crate::store::Store;

/// One roster group: a main agent and its sub-agents. The core has no
/// main/sub relation yet (see [`AgentUnavailable::SubAgent`]), so every
/// named agent is a main agent with no sub-agents.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentGroup<'a> {
    /// The main agent.
    pub main: &'a AgentRecord,
    /// Its sub-agents; always empty at this core rev.
    pub subs: Vec<&'a AgentRecord>,
}

/// The roster, one group per agent, in anchor order.
pub fn roster_groups(store: &Store) -> Vec<AgentGroup<'_>> {
    store.agents().map(|main| AgentGroup { main, subs: Vec::new() }).collect()
}

/// Every feature this build shows as unavailable, with its reason key. A
/// screen lists these next to the disabled control.
pub const UNAVAILABLE: [AgentUnavailable; 10] = [
    AgentUnavailable::SubAgent,
    AgentUnavailable::Skills,
    AgentUnavailable::AutonomyHonoured,
    AgentUnavailable::PerToolApproval,
    AgentUnavailable::HostBinding,
    AgentUnavailable::AvatarUpload,
    AgentUnavailable::PerAgentKey,
    AgentUnavailable::Retire,
    AgentUnavailable::RuntimeUpdate,
    AgentUnavailable::CreateOnPhone,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_unavailable_reason_has_a_distinct_key() {
        let keys: std::collections::BTreeSet<&str> =
            UNAVAILABLE.iter().map(|reason| reason.key()).collect();
        assert_eq!(keys.len(), UNAVAILABLE.len());
    }

    #[test]
    fn an_empty_store_has_no_groups() {
        assert!(roster_groups(&Store::new()).is_empty());
    }
}
