// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Who a one-sentence request is addressed to.
//!
//! A request goes to an AI participant of the shown conversation: the one
//! named by a leading mention (`@Orbit draft the notes`), else the one the
//! user picked (command palette), else the conversation's first AI
//! participant. Without one, there is nobody to ask.

use ramflux_desktop_core::{ContextId, ParticipantId, ParticipantKind, Store};

/// An AI participant that can be asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    /// Participant.
    pub id: ParticipantId,
    /// Display name.
    pub name: String,
}

/// The AI participants of `context`, in member order.
pub fn agents(store: &Store, context: &ContextId) -> Vec<Agent> {
    let Some(context) = store.context(context) else { return Vec::new() };
    context
        .participants
        .iter()
        .filter_map(|id| store.participant(id))
        .filter(|p| p.kind == ParticipantKind::Agent)
        .map(|p| Agent { id: p.id.clone(), name: p.display.clone() })
        .collect()
}

/// Splits a leading `@Name` off `text` when it names one of `agents`
/// (case-insensitive; a multi-word name may be written without spaces).
/// Returns the agent and the rest of the request.
pub fn mention<'a>(agents: &'a [Agent], text: &str) -> Option<(&'a Agent, String)> {
    let rest = text.strip_prefix('@')?;
    let word_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let word = rest[..word_end].trim_end_matches([',', ':']);
    let squash =
        |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase();
    let agent = agents.iter().find(|a| squash(&a.name) == word.to_lowercase())?;
    Some((agent, rest[word_end..].trim().to_owned()))
}

/// The agent a request goes to and the request text.
pub fn resolve<'a>(
    agents: &'a [Agent],
    picked: Option<&ParticipantId>,
    text: &str,
) -> Option<(&'a Agent, String)> {
    if let Some(found) = mention(agents, text) {
        return Some(found);
    }
    let agent =
        picked.and_then(|id| agents.iter().find(|a| &a.id == id)).or_else(|| agents.first())?;
    Some((agent, text.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agents() -> Vec<Agent> {
        vec![
            Agent { id: ParticipantId::new("p-orbit"), name: "Orbit".into() },
            Agent { id: ParticipantId::new("p-ledger"), name: "Ledger Bot".into() },
        ]
    }

    #[test]
    fn a_leading_mention_picks_the_agent_and_is_removed() {
        let agents = agents();
        let (agent, text) = mention(&agents, "@orbit, draft the notes").expect("mention");
        assert_eq!((agent.id.as_str(), text.as_str()), ("p-orbit", "draft the notes"));
        let (agent, _) = mention(&agents, "@LedgerBot check").expect("squashed name");
        assert_eq!(agent.id.as_str(), "p-ledger");
        assert!(mention(&agents, "@nobody hi").is_none());
        assert!(mention(&agents, "hi @orbit").is_none(), "only a leading mention addresses");
    }

    #[test]
    fn without_a_mention_the_picked_then_the_first_agent_is_asked() {
        let agents = agents();
        let picked = ParticipantId::new("p-ledger");
        assert_eq!(
            resolve(&agents, Some(&picked), "x").map(|(a, _)| a.id.as_str()),
            Some("p-ledger")
        );
        assert_eq!(resolve(&agents, None, "x").map(|(a, _)| a.id.as_str()), Some("p-orbit"));
        assert!(resolve(&[], None, "x").is_none());
    }
}
