// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The daemon side of exporting an AI participant's credentials, for a mock
//! source: the export is one-shot (a second one is refused as gone), a test
//! may script the next refusal, and nothing is written anywhere (the answer
//! only says where the bundle would be).

use std::collections::BTreeSet;

use ramflux_desktop_core::{
    AccountId, AgentExport, AgentExportPhase, AgentExportRefusal, DomainEvent, Effect, methods,
};

/// The mock daemon's credential exports.
#[derive(Debug, Default)]
pub struct MockAgents {
    exported: BTreeSet<String>,
    refusal: Option<AgentExportRefusal>,
}

impl MockAgents {
    /// The next export is refused with `refusal`.
    pub fn refuse_next_export(&mut self, refusal: AgentExportRefusal) {
        self.refusal = Some(refusal);
    }

    /// Answers an export; `None` for any other effect.
    pub fn answer(&mut self, effect: &Effect) -> Option<Vec<DomainEvent>> {
        if effect.method()? != methods::AGENT_EXPORT_CREDENTIALS {
            return None;
        }
        let body = effect.body()?;
        let text = |field: &str| {
            body.get(field).and_then(ramflux_desktop_core::JsonValue::as_str).unwrap_or_default()
        };
        let (agent, path) = (text("account").to_owned(), text("path").to_owned());
        let phase = match self.refusal.take() {
            Some(refusal) => AgentExportPhase::Refused(refusal),
            None if !self.exported.insert(agent.clone()) => {
                AgentExportPhase::Refused(AgentExportRefusal::CredentialsGone)
            }
            None => AgentExportPhase::Exported,
        };
        Some(vec![DomainEvent::AgentExportUpdated(AgentExport {
            account_id: AccountId::new(agent),
            path,
            phase,
        })])
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn export(agent: &str) -> Effect {
        Effect::Call {
            method: methods::AGENT_EXPORT_CREDENTIALS.to_owned(),
            body: json!({ "owner_id": "o", "account": agent, "path": "/x/bot.bundle" }),
        }
    }

    fn phase(events: &[DomainEvent]) -> AgentExportPhase {
        let Some(DomainEvent::AgentExportUpdated(export)) = events.first() else {
            panic!("an export update")
        };
        export.phase.clone()
    }

    #[test]
    fn the_export_is_one_shot_and_a_scripted_refusal_wins_once() {
        let mut mock = MockAgents::default();
        assert_eq!(phase(&mock.answer(&export("bot")).expect("ours")), AgentExportPhase::Exported);
        assert_eq!(
            phase(&mock.answer(&export("bot")).expect("ours")),
            AgentExportPhase::Refused(AgentExportRefusal::CredentialsGone)
        );
        mock.refuse_next_export(AgentExportRefusal::FileExists);
        assert_eq!(
            phase(&mock.answer(&export("other")).expect("ours")),
            AgentExportPhase::Refused(AgentExportRefusal::FileExists)
        );
        assert_eq!(
            phase(&mock.answer(&export("other")).expect("ours")),
            AgentExportPhase::Exported
        );
        assert!(mock.answer(&Effect::Call { method: "x".into(), body: json!({}) }).is_none());
    }
}
