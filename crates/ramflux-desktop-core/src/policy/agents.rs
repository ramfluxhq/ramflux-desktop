// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for AI participants an owner custodies (DESK-AGENT-01): creating
//! one in an attended ceremony and exporting its one-shot credentials.
//!
//! Both are human only and run on the owner's own account: an unlocked,
//! connected person's account of the daemon. The daemon enforces the same
//! rules (it is the authority); the policy keeps the desktop from sending a
//! request it knows is refused, and from spending the single confirmation
//! attempt on empty words. The desktop never unlocks an AI participant and
//! never holds the passphrase the daemon made for it.

use serde_json::json;

use super::{Decision, IntentError, Policy};
use crate::ids::AccountId;
use crate::intent::{IntentKind, methods};
use crate::model::{
    AccountKind, AgentCustodyProblem, AgentExportRefusal, CHALLENGE_WORDS, CeremonyPhase,
    LockState, Networking,
};
use crate::store::Store;

const WHAT: &str = "create an AI participant or export its credentials";

/// Why `custodian` cannot custody a new AI participant, if it cannot: it
/// must be an unlocked, connected person's account the daemon lists.
///
/// # Errors
///
/// The [`AgentCustodyProblem`] that applies first.
pub fn check_agent_custodian(
    store: &Store,
    custodian: &AccountId,
) -> Result<(), AgentCustodyProblem> {
    let account = store.account(custodian).ok_or(AgentCustodyProblem::Unknown)?;
    if account.kind != AccountKind::HumanInteractive {
        return Err(AgentCustodyProblem::NotPerson);
    }
    if account.lock_state != LockState::Unlocked {
        return Err(AgentCustodyProblem::Locked);
    }
    if account.networking != Networking::Live {
        return Err(AgentCustodyProblem::NotConnected);
    }
    Ok(())
}

/// Why the credentials of `agent` cannot be exported by `owner` to `path`,
/// if they cannot.
///
/// # Errors
///
/// The [`AgentExportRefusal`] that applies first.
pub fn check_agent_export(
    store: &Store,
    owner: &AccountId,
    agent: &AccountId,
    path: &str,
) -> Result<(), AgentExportRefusal> {
    let path = path.trim();
    if !std::path::Path::new(path).is_absolute() || path.chars().any(char::is_control) {
        return Err(AgentExportRefusal::NoLocation);
    }
    check_agent_custodian(store, owner).map_err(|problem| match problem {
        AgentCustodyProblem::Unknown => AgentExportRefusal::NotOwnerSession,
        AgentCustodyProblem::Locked => AgentExportRefusal::OwnerLocked,
        AgentCustodyProblem::NotPerson => AgentExportRefusal::NotCustodian,
        AgentCustodyProblem::NotConnected => AgentExportRefusal::NotConnected,
    })?;
    let account = store.account(agent).ok_or(AgentExportRefusal::NotCustodian)?;
    if account.kind != AccountKind::HeadlessAi {
        return Err(AgentExportRefusal::NotCustodian);
    }
    // Where the custodian is known (this desktop made the account, or the
    // daemon reported it), another account cannot export.
    if store.custodian_of(agent).is_some_and(|custodian| custodian != owner) {
        return Err(AgentExportRefusal::NotCustodian);
    }
    if account.networking != Networking::Live {
        return Err(AgentExportRefusal::NotConnected);
    }
    Ok(())
}

/// Decides AI participant kinds; `None` for kinds of other areas.
pub(super) fn decide(p: &Policy<'_>, kind: &IntentKind) -> Option<Result<Decision, IntentError>> {
    Some(match kind {
        IntentKind::BeginAgentCeremony { label, custodian_id } => {
            p.human_only(WHAT).and_then(|()| {
                if !p.store.offers_born_v3_creation() {
                    return Err(IntentError::CeremonyNotOffered);
                }
                if label.trim().is_empty() {
                    return Err(IntentError::Empty("AI participant name"));
                }
                check_agent_custodian(p.store, custodian_id).map_err(IntentError::AgentCustody)?;
                if p.store.ceremony().is_some_and(|ceremony| ceremony.is_active()) {
                    return Err(IntentError::CeremonyPhase);
                }
                Ok(Decision::Call(
                    methods::CUSTODY_BEGIN,
                    json!({
                        "display_name": label.trim(),
                        "principal_kind": "headless_ai",
                        "custodian_account": custodian_id,
                    }),
                ))
            })
        }
        IntentKind::ConfirmAgentCeremony { words } => p.human_only(WHAT).and_then(|()| {
            let asked = matches!(
                p.store.ceremony().map(|c| &c.phase),
                Some(CeremonyPhase::Challenged { .. })
            );
            if !asked {
                return Err(IntentError::CeremonyPhase);
            }
            if words.len() != CHALLENGE_WORDS || words.iter().any(|word| word.is_empty()) {
                return Err(IntentError::CeremonyWords);
            }
            Ok(Decision::SecretCall(methods::CUSTODY_CONFIRM, json!({})))
        }),
        IntentKind::ExportAgentCredentials { owner_id, account_id, path } => {
            p.human_only(WHAT).and_then(|()| {
                if !p.store.offers_born_v3_creation() {
                    return Err(IntentError::CeremonyNotOffered);
                }
                check_agent_export(p.store, owner_id, account_id, path)
                    .map_err(IntentError::AgentExport)?;
                Ok(Decision::Call(
                    methods::AGENT_EXPORT_CREDENTIALS,
                    json!({
                        "owner_id": owner_id,
                        "account": account_id,
                        "path": path.trim(),
                    }),
                ))
            })
        }
        _ => return None,
    })
}
