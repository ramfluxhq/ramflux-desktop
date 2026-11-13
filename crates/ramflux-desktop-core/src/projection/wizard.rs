// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Agent configuration review against an explicit host-provided catalog.
//! This validates a draft; it never creates credentials or grants authority.

use crate::{AccountId, AgentCustodyProblem, IntentKind, Store, check_agent_custodian};
use std::collections::BTreeSet;

/// Maximum number of tools/scopes in one local configuration draft.
pub const SELECTION_LIMIT: usize = 128;
/// Maximum bytes in a local display label or catalog identifier.
pub const FIELD_LIMIT: usize = 256;

/// Runtime and allowed choices reported by the host; no inferred defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RuntimeOffer {
    /// Stable host runtime identifier.
    pub id: String,
    /// Tool identifiers this runtime offers.
    pub tools: BTreeSet<String>,
    /// Scope identifiers this runtime offers.
    pub scopes: BTreeSet<String>,
}

impl RuntimeOffer {
    /// Constructs a runtime catalog entry. Selection is still validated later.
    #[must_use]
    pub fn new(id: String, tools: BTreeSet<String>, scopes: BTreeSet<String>) -> Self {
        Self { id, tools, scopes }
    }
}

/// User-editable draft. Contains no passphrase, seed, token or credential bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AgentDraft {
    /// Owner of the new participant.
    pub owner: AccountId,
    /// Display label.
    pub name: String,
    /// Host runtime identifier explicitly chosen by the owner.
    pub runtime: String,
    /// Explicit tool selections; empty grants nothing.
    pub tools: BTreeSet<String>,
    /// Explicit scope selections; empty grants nothing.
    pub scopes: BTreeSet<String>,
}

impl AgentDraft {
    /// Starts an empty configuration for `owner` with no ambient permissions.
    #[must_use]
    pub fn new(owner: AccountId) -> Self {
        Self {
            owner,
            name: String::new(),
            runtime: String::new(),
            tools: BTreeSet::new(),
            scopes: BTreeSet::new(),
        }
    }

    /// Validates review data against current custody and runtime facts.
    ///
    /// # Errors
    /// Refuses stale custody, missing/oversized fields, unknown runtimes and unoffered permissions.
    pub fn validate(&self, store: &Store, offers: &[RuntimeOffer]) -> Result<(), DraftError> {
        check_agent_custodian(store, &self.owner).map_err(DraftError::Custody)?;
        if valid_field(&self.name)
            && self.runtime.is_empty()
            && self.tools.is_empty()
            && self.scopes.is_empty()
        {
            return Ok(());
        }
        self.validate_selections(offers)
    }

    fn validate_selections(&self, offers: &[RuntimeOffer]) -> Result<(), DraftError> {
        if !valid_field(&self.name) || !valid_field(&self.runtime) {
            return Err(DraftError::Field);
        }
        if self.tools.len() > SELECTION_LIMIT || self.scopes.len() > SELECTION_LIMIT {
            return Err(DraftError::Limit);
        }
        if self.tools.iter().chain(&self.scopes).any(|id| !valid_field(id)) {
            return Err(DraftError::Field);
        }
        let mut matches = offers.iter().filter(|offer| offer.id == self.runtime);
        let offer = matches.next().ok_or(DraftError::Runtime)?;
        if matches.next().is_some() {
            return Err(DraftError::Runtime);
        }
        if !self.tools.is_subset(&offer.tools) || !self.scopes.is_subset(&offer.scopes) {
            return Err(DraftError::Permission);
        }
        Ok(())
    }

    /// Returns the existing identity-creation intent after current-state review.
    /// Configured drafts are unavailable until the backend can carry every selection.
    ///
    /// # Errors
    /// Returns validation errors or [`DraftError::ConfigurationUnavailable`]; no
    /// runtime/tool/scope selection is silently discarded.
    pub fn begin_identity(
        &self,
        store: &Store,
        offers: &[RuntimeOffer],
    ) -> Result<IntentKind, DraftError> {
        self.validate(store, offers)?;
        if !self.runtime.is_empty() || !self.tools.is_empty() || !self.scopes.is_empty() {
            return Err(DraftError::ConfigurationUnavailable);
        }
        Ok(IntentKind::BeginAgentCeremony {
            label: self.name.trim().to_owned(),
            custodian_id: self.owner.clone(),
        })
    }
}

fn valid_field(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= FIELD_LIMIT && !value.chars().any(char::is_control)
}

/// Why the configuration cannot proceed to review/identity creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DraftError {
    /// The backend cannot apply runtime/tools/scopes; the draft must be retained.
    #[error("agent configuration is unavailable in the backend")]
    ConfigurationUnavailable,
    /// Owner is no longer eligible.
    #[error("agent custodian is unavailable: {0:?}")]
    Custody(AgentCustodyProblem),
    /// Empty or oversized display/catalog field.
    #[error("invalid agent configuration field")]
    Field,
    /// Too many permissions selected.
    #[error("agent configuration selection limit exceeded")]
    Limit,
    /// Runtime not offered, or catalog contains ambiguous duplicate identifiers.
    #[error("runtime is not uniquely offered")]
    Runtime,
    /// Selected tool or scope was not offered for the chosen runtime.
    #[error("permission is not offered by the runtime")]
    Permission,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Account, DomainEvent, EventEnvelope, LockState, Timestamp};

    fn owner_store() -> Store {
        let mut store = Store::new();
        let account:Account=serde_json::from_value(serde_json::json!({"id":"owner","label":"Owner","lock_state":"Unlocked","networking":"Live"})).expect("test: connected human owner");
        store
            .apply(&EventEnvelope::new(1, Timestamp(0), DomainEvent::AccountUpdated(account)))
            .expect("test: owner source fact");
        store
    }

    #[test]
    fn creation_never_accepts_then_drops_configuration() {
        let mut store = owner_store();
        let mut draft = AgentDraft::new(AccountId::new("owner"));
        draft.name = "Research helper".into();
        assert_eq!(
            draft.validate(&store, &[]),
            Ok(()),
            "identity-only review must validate current custody without inventing a runtime"
        );
        assert!(
            matches!(draft.begin_identity(&store,&[]),Ok(IntentKind::BeginAgentCeremony{label,custodian_id}) if label=="Research helper" && custodian_id==draft.owner),
            "identity-only creation carries every selected field"
        );
        let offers = vec![RuntimeOffer::new(
            "host".into(),
            BTreeSet::from(["read".into()]),
            BTreeSet::new(),
        )];
        draft.runtime = "host".into();
        draft.tools.insert("read".into());
        assert_eq!(
            draft.validate(&store, &offers),
            Ok(()),
            "the configured draft is structurally valid against the offered catalog"
        );
        assert!(
            matches!(
                draft.begin_identity(&store, &offers),
                Err(DraftError::ConfigurationUnavailable)
            ),
            "creation must preserve configured drafts rather than drop their permissions"
        );
        draft.tools.clear();
        assert!(
            matches!(
                draft.begin_identity(&store, &offers),
                Err(DraftError::ConfigurationUnavailable)
            ),
            "even a runtime-only choice cannot be silently discarded"
        );
        let mut account = store.account(&draft.owner).cloned().expect("test: known owner");
        account.lock_state = LockState::Locked;
        store
            .apply(&EventEnvelope::new(2, Timestamp(1), DomainEvent::AccountUpdated(account)))
            .expect("test: owner locks before review");
        assert_eq!(
            draft.validate(&store, &offers),
            Err(DraftError::Custody(AgentCustodyProblem::Locked)),
            "public validation must recheck custody after the draft was displayed"
        );
    }

    #[test]
    fn configuration_cannot_expand_catalog_permissions_or_choose_defaults() {
        let mut draft = AgentDraft::new(AccountId::new("owner"));
        let offers = vec![RuntimeOffer::new(
            "host".into(),
            BTreeSet::from(["read".into()]),
            BTreeSet::from(["folder".into()]),
        )];
        assert_eq!(
            draft.validate_selections(&offers),
            Err(DraftError::Field),
            "wizard must start without a runtime or permissions"
        );
        draft.name = "Research helper".into();
        draft.runtime = "host".into();
        assert_eq!(
            draft.validate_selections(&offers),
            Ok(()),
            "an empty permission set grants nothing and is valid"
        );
        draft.tools.insert("execute".into());
        assert_eq!(
            draft.validate_selections(&offers),
            Err(DraftError::Permission),
            "tools cannot exceed the runtime catalog"
        );
        draft.tools.clear();
        draft.scopes.insert("all files".into());
        assert_eq!(
            draft.validate_selections(&offers),
            Err(DraftError::Permission),
            "scopes cannot exceed the runtime catalog"
        );
        draft.scopes.clear();
        assert_eq!(
            draft.validate_selections(&[offers[0].clone(), offers[0].clone()]),
            Err(DraftError::Runtime),
            "ambiguous runtime identifiers must fail closed"
        );
    }
}
