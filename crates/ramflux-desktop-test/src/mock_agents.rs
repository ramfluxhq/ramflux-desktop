// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The daemon side of named agents (AGENT-IDENTITY-01) for a mock source:
//! a creation waits for the phone until a test scripts its custody state, an
//! edit or an access change replaces the record, a delete retires it, and
//! the PRX runtime answers in process. Nothing is written anywhere.

use std::collections::BTreeMap;

use ramflux_desktop_core::{
    AgentAccess, AgentCreation, AgentCreationState, AgentHostState, AgentHostStatus,
    AgentLifecycle, AgentRecord, AgentRefusal, AgentRosterState, AgentRuntimeAction, DomainEvent,
    Effect, methods,
};
use serde_json::Value;

/// The mock daemon's agents: records by anchor, and the state the next
/// creation answer reports.
#[derive(Debug)]
pub struct MockAgents {
    records: BTreeMap<String, AgentRecord>,
    custody_state: String,
    next_request: u64,
    host: AgentHostStatus,
}

impl Default for MockAgents {
    fn default() -> Self {
        Self {
            records: BTreeMap::new(),
            custody_state: "authorization_pending".to_owned(),
            next_request: 1,
            host: AgentHostStatus {
                state: AgentHostState::Absent,
                prx_version: None,
                refusal: None,
                state_dir: None,
                applied: false,
                nonce: 0,
            },
        }
    }
}

impl MockAgents {
    /// The custody state the next creation status reports (for example
    /// `complete`, `refused` or `expired`, as the daemon names them).
    pub fn script_custody_state(&mut self, state: &str) {
        self.custody_state = state.to_owned();
    }

    /// Adds a record, as the daemon would list it.
    pub fn insert_record(&mut self, record: AgentRecord) {
        self.records.insert(record.agent_anchor.clone(), record);
    }

    /// Answers one effect; `None` for effects this mock does not own.
    pub fn answer(&mut self, effect: &Effect) -> Option<Vec<DomainEvent>> {
        if let Effect::AgentRuntime { action, nonce, .. } = effect {
            return Some(vec![self.runtime(*action, *nonce)]);
        }
        let method = effect.method()?;
        let body = effect.body()?;
        Some(match method {
            methods::AGENT_CREATE => vec![self.create()],
            methods::AGENT_OPERATION_GET => {
                let request_id = text(body, "operation_id").to_owned();
                vec![DomainEvent::AgentCreationUpdated(self.creation_status(request_id))]
            }
            methods::AGENT_LIST => vec![DomainEvent::AgentsListed {
                records: self.records.values().cloned().collect(),
                journal_cursor: "mock-cursor-1".to_owned(),
            }],
            methods::AGENT_PERSONA_EDIT => self.edit(body, |record| {
                if let Some(persona) = body.get("persona") {
                    record.persona = serde_json::from_value(persona.clone()).ok()?;
                }
                Some(())
            }),
            methods::AGENT_ACCESS_SET => {
                let access: Option<AgentAccess> =
                    body.get("access").and_then(|a| serde_json::from_value(a.clone()).ok());
                self.edit(body, |record| {
                    record.access = access.clone()?;
                    Some(())
                })
            }
            methods::AGENT_DELETE => self.edit(body, |record| {
                record.lifecycle = AgentLifecycle::Retired;
                Some(())
            }),
            methods::AGENT_JOB_PAUSE | methods::AGENT_JOB_RESUME | methods::AGENT_JOB_REMOVE => {
                Vec::new()
            }
            methods::AGENT_JOB_LIST => vec![DomainEvent::AgentJobsUpdated {
                agent: text(body, "agent_anchor").to_owned(),
                jobs: Vec::new(),
            }],
            methods::AGENT_APPROVAL_DECIDE => {
                vec![DomainEvent::AgentApprovalsUpdated(Vec::new())]
            }
            _ => return None,
        })
    }

    fn create(&mut self) -> DomainEvent {
        let request_id = format!("mock-req-{}", self.next_request);
        self.next_request += 1;
        DomainEvent::AgentCreationUpdated(AgentCreation {
            request_id,
            request_hash: "mock-hash".to_owned(),
            state: AgentCreationState::PendingPhone,
            agent_anchor: None,
            expires_at: Some(4_102_444_800),
        })
    }

    fn creation_status(&self, request_id: String) -> AgentCreation {
        let state = AgentCreationState::from_custody_state(&self.custody_state);
        let agent_anchor = (state == AgentCreationState::Created).then(|| "mock-agent".to_owned());
        AgentCreation {
            request_id,
            request_hash: "mock-hash".to_owned(),
            state,
            agent_anchor,
            expires_at: None,
        }
    }

    /// Applies `change` to the record named in the body and answers with it,
    /// or with a refusal of the roster when it is unknown.
    fn edit(
        &mut self,
        body: &Value,
        change: impl FnOnce(&mut AgentRecord) -> Option<()>,
    ) -> Vec<DomainEvent> {
        let anchor = text(body, "agent_anchor").to_owned();
        let Some(record) = self.records.get_mut(&anchor) else {
            return vec![DomainEvent::AgentRosterStateChanged(AgentRosterState::Failed(
                AgentRefusal::Unsupported,
            ))];
        };
        if change(record).is_none() {
            return vec![DomainEvent::AgentRosterStateChanged(AgentRosterState::Failed(
                AgentRefusal::PersonaInvalid,
            ))];
        }
        vec![DomainEvent::AgentRecordUpdated(Box::new(record.clone()))]
    }

    fn runtime(&mut self, action: AgentRuntimeAction, nonce: u64) -> DomainEvent {
        self.host.nonce = nonce;
        self.host.applied =
            matches!(action, AgentRuntimeAction::Install | AgentRuntimeAction::Restart);
        self.host.state = match action {
            AgentRuntimeAction::Install | AgentRuntimeAction::Restart => AgentHostState::Running,
            AgentRuntimeAction::Stop => AgentHostState::Stopped,
            AgentRuntimeAction::Uninstall => AgentHostState::Absent,
            AgentRuntimeAction::Status => self.host.state,
        };
        if self.host.state == AgentHostState::Running {
            self.host.prx_version = Some("mock".to_owned());
            self.host.state_dir = Some(std::path::PathBuf::from("/mock/prx"));
        }
        DomainEvent::AgentHostUpdated(self.host.clone())
    }
}

fn text<'a>(body: &'a Value, field: &str) -> &'a str {
    body.get(field).and_then(Value::as_str).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use ramflux_desktop_core::AgentPersona;
    use serde_json::json;

    use super::*;

    fn call(method: &str, body: Value) -> Effect {
        Effect::Call { method: method.to_owned(), body }
    }

    fn record() -> AgentRecord {
        AgentRecord {
            agent_anchor: "agent-1".into(),
            custodian_anchor: "owner".into(),
            persona: AgentPersona::default(),
            lifecycle: AgentLifecycle::Active,
            access: AgentAccess::owner_only(),
            role: "named".into(),
            base: json!({"id": "base-1"}),
            runtime_name: String::new(),
        }
    }

    #[test]
    fn a_creation_waits_for_the_phone_until_the_script_says_otherwise() {
        let mut mock = MockAgents::default();
        let events = mock.answer(&call(methods::AGENT_CREATE, json!({}))).expect("ours");
        let [DomainEvent::AgentCreationUpdated(created)] = events.as_slice() else {
            panic!("test: a creation update")
        };
        assert_eq!(created.state, AgentCreationState::PendingPhone);

        mock.script_custody_state("complete");
        let status = mock
            .answer(&call(methods::AGENT_OPERATION_GET, json!({"operation_id": "mock-req-1"})))
            .expect("ours");
        let [DomainEvent::AgentCreationUpdated(done)] = status.as_slice() else {
            panic!("test: a status update")
        };
        assert_eq!(done.state, AgentCreationState::Created);
        assert_eq!(done.agent_anchor.as_deref(), Some("mock-agent"));
    }

    #[test]
    fn an_edit_replaces_the_record_and_an_unknown_one_is_refused() {
        let mut mock = MockAgents::default();
        mock.insert_record(record());
        let edit = call(
            methods::AGENT_PERSONA_EDIT,
            json!({"agent_anchor": "agent-1", "persona": {"name": "N", "avatar": "", "responsibility": "R"}}),
        );
        let events = mock.answer(&edit).expect("ours");
        let [DomainEvent::AgentRecordUpdated(updated)] = events.as_slice() else {
            panic!("test: a record update")
        };
        assert_eq!(updated.persona.name, "N");

        let unknown = call(methods::AGENT_DELETE, json!({"agent_anchor": "nope"}));
        assert!(matches!(
            mock.answer(&unknown).expect("ours").as_slice(),
            [DomainEvent::AgentRosterStateChanged(AgentRosterState::Failed(_))]
        ));
    }

    #[test]
    fn runtime_actions_answer_in_process_and_other_methods_are_not_ours() {
        let mut mock = MockAgents::default();
        let effect = Effect::AgentRuntime {
            action: AgentRuntimeAction::Install,
            provider: "p".into(),
            model: "m".into(),
            key: None,
            nonce: 1,
        };
        assert!(matches!(
            mock.answer(&effect).expect("ours").as_slice(),
            [DomainEvent::AgentHostUpdated(status)] if status.state == AgentHostState::Running
        ));
        assert!(mock.answer(&call("x", json!({}))).is_none());
    }
}
