// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Intent validation and policy (rules R2 and R6).
//!
//! [`handle_intent`] reads the store, never mutates it, and turns a valid
//! intent into effects for the source. Each area module validates its kinds
//! and returns a [`Decision`]; this module turns decisions into effects and
//! stamps the audit pair (`proposed_by`, `confirmed_by`) on every body.
//!
//! - `conversation`: messages, context settings and objects.
//! - `tasks`: asking an AI participant, confirming its proposal, cancelling.
//! - `moments`: surface actions, approvals and inline moment decisions.
//! - `governance`: membership, roles and grant revocation.
//! - `contacts`: adding contacts and acting on contact links.
//! - `start`: starting a direct conversation with a contact.
//! - `groups`: creating and renaming a group.
//! - `calls`: placing, answering, declining and hanging up voice calls.
//! - `accounts`: entry (accounts, devices, nodes) and settings.
//! - `backup`: full backup and restore.
//! - `ceremony`: account creation with a recovery phrase.

mod accounts;
mod backup;
mod calls;
mod ceremony;
mod contacts;
mod conversation;
mod error;
mod governance;
mod groups;
mod moments;
mod start;
mod tasks;

use serde_json::{Value, json};

use crate::ids::{ContextId, MomentId, Timestamp};
use crate::intent::{Effect, Intent, IntentKind, SecretInput};
use crate::model::{Actor, Context, Moment, ParticipantKind, Risk};
use crate::store::Store;

pub use calls::{call_peer, check_start_call};
pub use contacts::{check_add_contact, check_add_contact_card, contact_action_current};
pub use error::IntentError;
pub use groups::{
    GroupProblem, MAX_GROUP_MEMBERS, MAX_GROUP_TITLE_LEN, check_create_group, check_group_title,
};
pub use start::check_start_conversation;

/// What an area decided about one intent.
pub(crate) enum Decision {
    /// Call `method` with `body`.
    Call(&'static str, Value),
    /// Call `method` with `body` and the intent's secrets.
    SecretCall(&'static str, Value),
    /// The intent (calling `method`) needs the attended human's hold first.
    Confirm(&'static str),
}

/// What every area needs to decide.
pub(crate) struct Policy<'a> {
    /// The store, read-only.
    pub store: &'a Store,
    /// Intent time.
    pub now: Timestamp,
    /// Who proposed the intent.
    pub proposer: &'a Actor,
    /// The intent was proposed by an agent.
    pub is_agent: bool,
    /// The attended human completed the hold.
    pub human_confirmed: bool,
}

impl Policy<'_> {
    /// `Call`, or `Confirm` when `risk` needs a hold that has not happened.
    pub fn gated(&self, risk: Risk, method: &'static str, body: Value) -> Decision {
        if risk.requires_confirmation() && !self.human_confirmed {
            Decision::Confirm(method)
        } else {
            Decision::Call(method, body)
        }
    }

    /// Refuses the intent for agents.
    pub fn human_only(&self, what: &'static str) -> Result<(), IntentError> {
        if self.is_agent { Err(IntentError::AgentForbidden(what)) } else { Ok(()) }
    }

    /// The context, or an error.
    pub fn context(&self, id: &ContextId) -> Result<&Context, IntentError> {
        self.store.context(id).ok_or_else(|| IntentError::UnknownContext(id.clone()))
    }

    /// The moment, or an error.
    pub fn moment(&self, id: &MomentId) -> Result<&Moment, IntentError> {
        self.store.moment(id).ok_or_else(|| IntentError::UnknownMoment(id.clone()))
    }
}

/// Validates `intent` against `store` at time `now` and returns the effects to submit.
///
/// Policy: approving a `High` or `Critical` grant, invoking a `High` or
/// `Critical` surface action, a high-risk governance operation (`Kick`,
/// `Ban`, `SetRole`), revealing masked content, revoking a grant, standing
/// approval or device, blocking or removing a contact, and confirming a
/// trust-on-first-use node all need the attended human's hold: without
/// `confirmed_by` they yield [`Effect::RequireConfirmation`]. Agent proposals
/// follow the same rules. Agents may never unlock, create, activate or
/// restore an account, rotate its passphrase, export a backup, back up or
/// restore in full, create an account with a recovery phrase, sync or revoke
/// devices, confirm a node, verify a safety number, answer a handoff, add a
/// contact, share the contact card, start a conversation, or create a group. Grants whose expiry is at or before `now` cannot be approved or
/// denied.
///
/// Every call body carries `proposed_by` and `confirmed_by`. An intent whose
/// method the backend declared unsupported is refused before any hold.
///
/// # Errors
///
/// Returns [`IntentError`] when the intent references unknown entities, targets
/// a moment in the wrong kind or state, or is not allowed for the actor.
pub fn handle_intent(
    store: &Store,
    intent: Intent,
    now: Timestamp,
) -> Result<Vec<Effect>, IntentError> {
    let proposed_by = actor_json(store, &intent.proposed_by)?;
    let confirmed_by = json!(intent.confirmed_by);
    let policy = Policy {
        store,
        now,
        proposer: &intent.proposed_by,
        is_agent: !matches!(intent.proposed_by, Actor::Human),
        human_confirmed: intent
            .confirmed_by
            .as_ref()
            .is_some_and(|c| c.held_ms() >= crate::model::MIN_HOLD_MS),
    };
    let decision = conversation::decide(&policy, &intent.kind)
        .or_else(|| tasks::decide(&policy, &intent.kind))
        .or_else(|| moments::decide(&policy, &intent.kind))
        .or_else(|| governance::decide(&policy, &intent.kind))
        .or_else(|| contacts::decide(&policy, &intent.kind))
        .or_else(|| start::decide(&policy, &intent.kind))
        .or_else(|| groups::decide(&policy, &intent.kind))
        .or_else(|| calls::decide(&policy, &intent.kind))
        .or_else(|| accounts::decide(&policy, &intent.kind))
        .or_else(|| backup::decide(&policy, &intent.kind))
        .or_else(|| ceremony::decide(&policy, &intent.kind))
        .unwrap_or_else(|| unreachable!("every intent kind belongs to one area"))?;
    let method = match &decision {
        Decision::Call(method, _) | Decision::SecretCall(method, _) | Decision::Confirm(method) => {
            *method
        }
    };
    if let Some(reason) = store.unsupported_reason(method) {
        return Err(IntentError::UnsupportedByBackend { method, reason: reason.to_owned() });
    }
    let stamp = |mut body: Value| {
        if let Value::Object(map) = &mut body {
            map.insert("proposed_by".to_owned(), proposed_by.clone());
            map.insert("confirmed_by".to_owned(), confirmed_by.clone());
        }
        body
    };
    Ok(vec![match decision {
        Decision::Call(method, body) => {
            Effect::Call { method: method.to_owned(), body: stamp(body) }
        }
        Decision::SecretCall(method, body) => Effect::SecretCall {
            method: method.to_owned(),
            body: stamp(body),
            secrets: take_secrets(intent.kind),
        },
        Decision::Confirm(_) => Effect::RequireConfirmation { intent: Box::new(intent) },
    }])
}

/// Moves the secrets out of a secret-carrying intent, in method order.
fn take_secrets(kind: IntentKind) -> Vec<SecretInput> {
    match kind {
        IntentKind::Unlock { secret, .. }
        | IntentKind::CreateAccount { secret, .. }
        | IntentKind::RestoreBackup { secret, .. }
        | IntentKind::ExportBackup { secret, .. }
        | IntentKind::CreateBackup { secret, .. }
        | IntentKind::VerifyRestore { secret, .. } => vec![secret],
        IntentKind::ConfirmCeremony { words } => words,
        IntentKind::ActivateDevice { code, .. } => vec![code],
        IntentKind::RotatePassphrase { current, next, .. } => vec![current, next],
        _ => Vec::new(),
    }
}

fn actor_json(store: &Store, actor: &Actor) -> Result<Value, IntentError> {
    match actor {
        Actor::Human => Ok(json!("Human")),
        Actor::Agent(id) => {
            let known = store.participant(id).is_some_and(|p| p.kind == ParticipantKind::Agent);
            if known {
                Ok(json!({ "Agent": id }))
            } else {
                Err(IntentError::UnknownAgent(id.clone()))
            }
        }
        // Another member of a conversation is shown as a task's requester;
        // it never proposes an intent on this desktop.
        Actor::Member(id) => Err(IntentError::UnknownAgent(id.clone())),
    }
}
