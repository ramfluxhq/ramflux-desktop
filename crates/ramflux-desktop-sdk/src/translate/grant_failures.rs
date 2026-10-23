// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Fail-closed grant errors.
//!
//! The daemon refuses a decision with an error frame (`code`, `message`).
//! The classes that close a grant for good become `GrantFailed`, next to the
//! `EffectFailed` every refusal produces:
//!
//! | daemon frame | failure |
//! |---|---|
//! | `RemoteAppApprovalRequired` | the request needs a signed approval |
//! | `SignatureVerificationFailed` | the approval signature was rejected |
//! | `GrantInvalidated` | expired (past its expiry) or invalidated |
//! | `ValidationFailed: approval not found`, first decision sent | already used or decided elsewhere |
//! | the same, after a decision was already sent from here | replay |
//!
//! Transport errors and timeouts close nothing (the outcome is unknown and
//! the next approvals read tells). An approval the daemon answers with
//! arguments other than the ones reviewed, or already expired, fails closed
//! too, as does a pending request whose arguments change. Nothing here ever
//! sends a decision again.

use ramflux_desktop_core::{
    DomainEvent, FailureCode, GrantFailure, GrantId, GrantState, Timestamp, methods,
};
use serde_json::Value;

use super::{Translator, seconds, text};

/// What a grant was bound to when it was requested.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Binding {
    /// Arguments hash shown to the user.
    pub args_hash: String,
    /// Expiry.
    pub expiry: Option<Timestamp>,
    /// Decisions sent from here.
    pub decisions: u32,
    /// Already reported as failed.
    pub failed: bool,
}

impl Translator {
    /// Remembers a decision being sent, to tell a replay from a first use.
    pub fn decision_sent(&mut self, method: &str, body: &Value) {
        if matches!(method, methods::GRANT_APPROVE | methods::GRANT_DENY) {
            let binding = self.bindings.entry(GrantId::new(text(body, "grant_id"))).or_default();
            binding.decisions = binding.decisions.saturating_add(1);
        }
    }

    /// Records what a newly requested grant is bound to.
    pub(super) fn bind(&mut self, id: &GrantId, args_hash: &str, expiry: Option<Timestamp>) {
        let binding = self.bindings.entry(id.clone()).or_default();
        binding.args_hash = args_hash.to_owned();
        binding.expiry = expiry;
    }

    /// `GrantFailed` once per grant.
    fn fail(&mut self, id: &GrantId, failure: GrantFailure) -> Vec<DomainEvent> {
        let Some(binding) = self.bindings.get_mut(id) else { return Vec::new() };
        if binding.failed || !self.grants.contains_key(id) {
            return Vec::new();
        }
        binding.failed = true;
        let mut events = vec![DomainEvent::GrantFailed { grant_id: id.clone(), failure }];
        events.extend(self.intervention_failed(id));
        events
    }

    fn expired(&self, id: &GrantId, now: Timestamp) -> bool {
        self.bindings.get(id).and_then(|b| b.expiry).is_some_and(|at| at <= now)
    }

    /// The grant failure an error frame on a decision means, if it closes it.
    pub fn effect_failed(
        &mut self,
        method: &str,
        body: &Value,
        code: &FailureCode,
        message: &str,
        now: Timestamp,
    ) -> Vec<DomainEvent> {
        if method == methods::ATTACH_OBJECT {
            return self.attachment_failed(body, message);
        }
        if !matches!(method, methods::GRANT_APPROVE | methods::GRANT_DENY) {
            return Vec::new();
        }
        let id = GrantId::new(text(body, "grant_id"));
        let FailureCode::Daemon(code) = code else { return Vec::new() };
        let failure = match code.as_str() {
            "RemoteAppApprovalRequired" => GrantFailure::SignedApprovalRequired,
            "SignatureVerificationFailed" => GrantFailure::SignatureRejected,
            "GrantInvalidated" if self.expired(&id, now) => GrantFailure::Expired,
            "GrantInvalidated" => GrantFailure::Invalidated,
            "ValidationFailed" if message.contains("approval not found") => {
                let sent = self.bindings.get(&id).map_or(0, |b| b.decisions);
                let decided = self.grants.get(&id).is_some_and(|s| *s != GrantState::Pending);
                if sent > 1 || decided {
                    GrantFailure::Replay
                } else if self.expired(&id, now) {
                    GrantFailure::Expired
                } else {
                    GrantFailure::AlreadyUsed
                }
            }
            _ => return Vec::new(),
        };
        self.fail(&id, failure)
    }

    /// Checks the daemon's approval answer against what was reviewed.
    pub(super) fn approval_answer(
        &mut self,
        id: &GrantId,
        reviewed: &str,
        answer: &Value,
        now: Timestamp,
    ) -> Vec<DomainEvent> {
        let signed = &answer["signing_body"];
        let bound = signed["arguments_hash"].as_str().unwrap_or_default();
        if !reviewed.is_empty() && !bound.is_empty() && bound != reviewed {
            return self.fail(id, GrantFailure::ArgumentsMismatch);
        }
        if seconds(&signed["expires_at"]).is_some_and(|at| at <= now) {
            return self.fail(id, GrantFailure::Expired);
        }
        Vec::new()
    }

    /// A pending request pushed again with other arguments than the ones
    /// shown is closed: the user never reviewed them.
    pub(super) fn approval_changed(&mut self, id: &GrantId, record: &Value) -> Vec<DomainEvent> {
        let pushed = record["details"]["arguments_hash"].as_str().unwrap_or_default();
        let shown = self.bindings.get(id).map(|b| b.args_hash.clone()).unwrap_or_default();
        if !pushed.is_empty() && !shown.is_empty() && pushed != shown {
            self.fail(id, GrantFailure::ArgumentsMismatch)
        } else {
            Vec::new()
        }
    }
}
