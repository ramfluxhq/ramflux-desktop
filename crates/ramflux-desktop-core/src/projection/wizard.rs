// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The new-agent draft (the five steps of the wizard). It validates what
//! the owner chose and builds the `agent.create` request; it never invents
//! a provider, a model or a limit: a value the owner did not choose stays
//! unset and blocks Create.

use crate::model::{AgentAccess, AgentConfig, AgentCreateRequest, AgentPersona, AgentTools};
use crate::{AccountId, AgentCustodyProblem, Store, check_agent_custodian};

/// Maximum bytes in one text field of the draft.
pub const FIELD_LIMIT: usize = 256;
/// Maximum bytes in the system prompt.
pub const PROMPT_LIMIT: usize = 8192;

/// The wizard's steps, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardStep {
    /// Name, avatar preset and responsibility.
    Basics,
    /// The PRX runtime: its status and the provider and model.
    Runtime,
    /// Autonomy, tools and limits.
    Capabilities,
    /// Who may reach the agent.
    Conversations,
    /// The review and Create.
    Review,
}

impl WizardStep {
    /// Every step, in order.
    pub const ALL: [Self; 5] =
        [Self::Basics, Self::Runtime, Self::Capabilities, Self::Conversations, Self::Review];
}

/// The draft the owner is filling in. Contains no key, passphrase or
/// credential: the provider key goes to the runtime, never here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDraft {
    /// The owner's human account on this daemon.
    pub owner: AccountId,
    /// The persona.
    pub persona: AgentPersona,
    /// The provider id, free text until the PRX catalog exists.
    pub provider: String,
    /// The model id, free text until the PRX catalog exists.
    pub model: String,
    /// The autonomy token. `full` is the only value the runtime publishes.
    pub autonomy: String,
    /// The tools; `None` until the owner chooses.
    pub tools: Option<AgentTools>,
    /// An optional system prompt; the runtime does not apply it yet.
    pub prompt: Option<String>,
    /// Concurrent turns; `None` until the owner chooses.
    pub max_concurrent_turns: Option<u32>,
    /// Cost per day in cents; `None` until the owner chooses.
    pub max_cost_per_day_cents: Option<u64>,
    /// Who may reach the agent (owner only until the owner changes it).
    pub access: AgentAccess,
}

impl AgentDraft {
    /// A new draft for `owner`: nothing chosen except the owner-only access.
    #[must_use]
    pub fn new(owner: AccountId) -> Self {
        Self {
            owner,
            persona: AgentPersona::default(),
            provider: String::new(),
            model: String::new(),
            autonomy: "full".to_owned(),
            tools: None,
            prompt: None,
            max_concurrent_turns: None,
            max_cost_per_day_cents: None,
            access: AgentAccess::owner_only(),
        }
    }

    /// Checks the draft against the current facts: the owner's account and
    /// every chosen field.
    ///
    /// # Errors
    ///
    /// [`DraftError::Custody`] when the owner cannot own an agent now, and
    /// [`DraftError::Field`] with the first field that is missing or too
    /// long.
    pub fn validate(&self, store: &Store) -> Result<(), DraftError> {
        check_agent_custodian(store, &self.owner).map_err(DraftError::Custody)?;
        let text = [
            ("name", &self.persona.name),
            ("avatar", &self.persona.avatar),
            ("responsibility", &self.persona.responsibility),
            ("provider", &self.provider),
            ("model", &self.model),
            ("autonomy", &self.autonomy),
        ];
        for (field, value) in text {
            if !valid_field(value) {
                return Err(DraftError::Field(field));
            }
        }
        if self.tools.is_none() {
            return Err(DraftError::Field("tools"));
        }
        if self.max_concurrent_turns.is_none() {
            return Err(DraftError::Field("max_concurrent_turns"));
        }
        if self.max_cost_per_day_cents.is_none() {
            return Err(DraftError::Field("max_cost_per_day_cents"));
        }
        if self.prompt.as_deref().is_some_and(|p| p.len() > PROMPT_LIMIT) {
            return Err(DraftError::Field("prompt"));
        }
        if !self.access.is_valid() {
            return Err(DraftError::Access);
        }
        Ok(())
    }

    /// The `agent.create` request for this draft, once it validates.
    ///
    /// # Errors
    ///
    /// The same errors as [`Self::validate`].
    pub fn request(&self, store: &Store) -> Result<AgentCreateRequest, DraftError> {
        self.validate(store)?;
        let (Some(tools), Some(turns), Some(cost)) =
            (self.tools, self.max_concurrent_turns, self.max_cost_per_day_cents)
        else {
            return Err(DraftError::Field("limits"));
        };
        Ok(AgentCreateRequest {
            persona: self.persona.clone(),
            config: AgentConfig {
                provider: self.provider.trim().to_owned(),
                model: self.model.trim().to_owned(),
                autonomy: self.autonomy.trim().to_owned(),
                tools,
                prompt: self.prompt.clone().filter(|p| !p.trim().is_empty()),
                max_concurrent_turns: turns,
                max_cost_per_day_cents: cost,
            },
            access: self.access.clone(),
        })
    }
}

fn valid_field(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= FIELD_LIMIT && !value.chars().any(char::is_control)
}

/// Why the draft cannot be created yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DraftError {
    /// The owner cannot own an agent now.
    #[error("the owner cannot own an agent: {0:?}")]
    Custody(AgentCustodyProblem),
    /// A field is missing, too long or not chosen yet.
    #[error("the agent field {0} is missing or invalid")]
    Field(&'static str),
    /// The access policy is not one the daemon accepts.
    #[error("the agent access policy is invalid")]
    Access,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Account, DomainEvent, EventEnvelope, LockState, Timestamp};

    fn owner_store() -> Store {
        let mut store = Store::new();
        let account: Account = serde_json::from_value(serde_json::json!({
            "id": "owner",
            "label": "Owner",
            "lock_state": "Unlocked",
            "networking": "Live"
        }))
        .expect("test: a connected owner account");
        store
            .apply(&EventEnvelope::new(1, Timestamp(0), DomainEvent::AccountUpdated(account)))
            .expect("test: owner source fact");
        store
    }

    fn complete(owner: &str) -> AgentDraft {
        let mut draft = AgentDraft::new(AccountId::new(owner));
        draft.persona = AgentPersona {
            name: "Research helper".into(),
            avatar: "preset:spark:teal".into(),
            responsibility: "Reads papers".into(),
        };
        draft.provider = "anthropic".into();
        draft.model = "model-x".into();
        draft.tools = Some(AgentTools::None);
        draft.max_concurrent_turns = Some(2);
        draft.max_cost_per_day_cents = Some(500);
        draft
    }

    #[test]
    fn nothing_is_defaulted_and_every_field_blocks_create() {
        let store = owner_store();
        let draft = AgentDraft::new(AccountId::new("owner"));
        assert_eq!(draft.validate(&store), Err(DraftError::Field("name")));
        let filled = complete("owner");
        assert_eq!(filled.validate(&store), Ok(()));
        let mut no_limits = filled.clone();
        no_limits.max_cost_per_day_cents = None;
        assert_eq!(
            no_limits.request(&store),
            Err(DraftError::Field("max_cost_per_day_cents")),
            "limits are sent always, so an unchosen limit blocks the request"
        );
    }

    #[test]
    fn the_request_carries_both_limits_and_never_a_host() {
        let store = owner_store();
        let request = complete("owner").request(&store).expect("test: complete draft");
        let value = serde_json::to_value(&request).expect("test: request serializes");
        assert_eq!(value["config"]["provider"], "anthropic");
        assert!(value["config"].get("host").is_none(), "a host binding is never sent");
        assert_eq!(request.config.tools.bus_list(), Vec::<&str>::new());
        assert_eq!(request.config.max_concurrent_turns, 2);
        assert_eq!(request.config.max_cost_per_day_cents, 500);
    }

    #[test]
    fn a_locked_owner_cannot_create_and_the_draft_is_kept() {
        let mut store = owner_store();
        let draft = complete("owner");
        let mut account = store.account(&draft.owner).cloned().expect("test: known owner");
        account.lock_state = LockState::Locked;
        store
            .apply(&EventEnvelope::new(2, Timestamp(1), DomainEvent::AccountUpdated(account)))
            .expect("test: owner locks");
        assert_eq!(draft.validate(&store), Err(DraftError::Custody(AgentCustodyProblem::Locked)));
        assert_eq!(draft.provider, "anthropic", "a refused draft keeps what the owner chose");
    }
}
