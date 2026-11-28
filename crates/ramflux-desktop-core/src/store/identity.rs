// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Contact identity and the security of direct conversations.
//!
//! A change event sets the contact's pending change (a newer one replaces an
//! older one), marks the identity `Changed` and adds one service line to
//! each direct conversation with the contact. An acceptance clears the
//! change and adds a service line; the state itself is read again by the
//! source. A security read keeps the conversation's latest security and adds
//! one service line per epoch of a fork recovery. A send refused for a
//! pending change is remembered per conversation until the identity is
//! decided. Nothing here accepts, sends or resends anything.

use super::Store;
use super::changes::{ApplyError, Changes};
use crate::ids::{ContextId, MomentId, ParticipantId, Timestamp};
use crate::model::{
    AcceptFailure, ContextKind, ConversationSecurity, IdentityChange, IdentityChangeKind,
    IdentityChangePayload, IdentityState, Lifecycle, Moment, MomentKind, SecurityNoticePayload,
};

impl Store {
    /// The latest security read of a direct conversation.
    pub fn security(&self, context: &ContextId) -> Option<&ConversationSecurity> {
        self.security.get(context)
    }

    /// Why the last acceptance for `participant` did not take effect, with
    /// the number of the failure (later failures have higher numbers) and
    /// the anchor the refused acceptance named, until the next change,
    /// acceptance or read of the identity.
    pub fn identity_failure(
        &self,
        participant: &ParticipantId,
    ) -> Option<(AcceptFailure, u64, Option<&str>)> {
        self.identity_failures
            .get(participant)
            .map(|(reason, seq, anchor)| (*reason, *seq, anchor.as_deref()))
    }

    /// The number of the latest acceptance failure recorded (0 for none): a
    /// failure is new to a view when its number is higher.
    pub fn identity_failure_seq(&self) -> u64 {
        self.identity_failure_seq
    }

    /// The contact whose identity change refused the last send in `context`
    /// (and the failed message), while that change waits.
    pub fn send_blocked(&self, context: &ContextId) -> Option<&(ParticipantId, Option<MomentId>)> {
        self.send_blocked.get(context)
    }

    /// The direct conversations with `participant`.
    fn direct_contexts(&self, participant: &ParticipantId) -> Vec<ContextId> {
        self.contexts
            .values()
            .filter(|c| c.kind == ContextKind::Direct && c.participants.contains(participant))
            .map(|c| c.id.clone())
            .collect()
    }

    /// Adds a service line unless one with the id exists (a replayed event).
    fn add_service_line(
        &mut self,
        at: Timestamp,
        moment: Moment,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        if self.moments.contains_key(&moment.id) {
            return Ok(());
        }
        self.add_moment(at, &moment, changes)
    }

    pub(super) fn identity_changed(
        &mut self,
        at: Timestamp,
        participant: &ParticipantId,
        change: IdentityChange,
        ai_flag: bool,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        let new_anchor = change.new_anchor.clone();
        let who = self
            .participants
            .get_mut(participant)
            .ok_or_else(|| ApplyError::UnknownParticipant(participant.clone()))?;
        who.identity = IdentityState::Changed;
        who.ai_flag = ai_flag;
        who.pending_change = Some(change);
        self.identity_failures.remove(participant);
        changes.participants.insert(participant.clone());
        for context in self.direct_contexts(participant) {
            changes.contexts.insert(context.clone());
            let moment = Moment {
                id: MomentId::new(format!("identity:changed:{participant}:{new_anchor}")),
                context_id: context,
                origin: participant.clone(),
                at,
                kind: MomentKind::IdentityChange(IdentityChangePayload {
                    participant_id: participant.clone(),
                    change: IdentityChangeKind::Changed,
                }),
                lifecycle: Lifecycle::Done,
                order: None,
            };
            self.add_service_line(at, moment, changes)?;
        }
        Ok(())
    }

    pub(super) fn identity_state_read(
        &mut self,
        participant: &ParticipantId,
        state: IdentityState,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        let who = self
            .participants
            .get_mut(participant)
            .ok_or_else(|| ApplyError::UnknownParticipant(participant.clone()))?;
        // A state that is not `Changed` ends any change waiting; `Changed`
        // keeps the one the event named (or none, until an event names it).
        let pending = if state.is_changed() { who.pending_change.clone() } else { None };
        if who.identity != state || who.pending_change != pending {
            who.identity = state;
            who.pending_change = pending;
            changes.participants.insert(participant.clone());
            // The conversation list's badge follows the state.
            for context in self.direct_contexts(participant) {
                changes.contexts.insert(context);
            }
        }
        if !state.is_changed() {
            self.identity_decided(participant, changes);
        }
        Ok(())
    }

    pub(super) fn identity_accepted(
        &mut self,
        at: Timestamp,
        participant: &ParticipantId,
        accepted: Option<&str>,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        let who = self
            .participants
            .get_mut(participant)
            .ok_or_else(|| ApplyError::UnknownParticipant(participant.clone()))?;
        // A newer change than the accepted one is still waiting: it stays
        // pending (and `Changed`) and is not announced as accepted.
        if let (Some(accepted), Some(pending)) = (accepted, who.pending_change.as_ref())
            && pending.new_anchor != accepted
        {
            return Ok(());
        }
        let anchor = who.pending_change.take().map(|c| c.new_anchor);
        // Until the source reads the state again it is no longer a pending
        // change; the first-seen state is the neutral one.
        if who.identity.is_changed() {
            who.identity = IdentityState::Tofu;
        }
        changes.participants.insert(participant.clone());
        self.identity_decided(participant, changes);
        let key = anchor.unwrap_or_else(|| at.0.to_string());
        for context in self.direct_contexts(participant) {
            changes.contexts.insert(context.clone());
            let moment = Moment {
                id: MomentId::new(format!("identity:accepted:{participant}:{key}")),
                context_id: context,
                origin: participant.clone(),
                at,
                kind: MomentKind::IdentityChange(IdentityChangePayload {
                    participant_id: participant.clone(),
                    change: IdentityChangeKind::Accepted,
                }),
                lifecycle: Lifecycle::Done,
                order: None,
            };
            self.add_service_line(at, moment, changes)?;
        }
        Ok(())
    }

    pub(super) fn identity_accept_failed(
        &mut self,
        participant: &ParticipantId,
        reason: AcceptFailure,
        attempted: Option<&str>,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        let who = self
            .participants
            .get_mut(participant)
            .ok_or_else(|| ApplyError::UnknownParticipant(participant.clone()))?;
        // The change the user looked at is gone (handled elsewhere): that one
        // is never offered again, but a newer change than the attempted one
        // stays pending to be reviewed.
        if reason == AcceptFailure::AlreadyHandled
            && who
                .pending_change
                .as_ref()
                .is_some_and(|c| attempted.is_none_or(|a| c.new_anchor == a))
        {
            who.pending_change = None;
            changes.participants.insert(participant.clone());
        }
        self.identity_failure_seq += 1;
        self.identity_failures.insert(
            participant.clone(),
            (reason, self.identity_failure_seq, attempted.map(str::to_owned)),
        );
        for context in self.direct_contexts(participant) {
            changes.contexts.insert(context);
        }
        changes.participants.insert(participant.clone());
        Ok(())
    }

    /// Forgets the refusal state of an identity that was decided.
    fn identity_decided(&mut self, participant: &ParticipantId, changes: &mut Changes) {
        if self.identity_failures.remove(participant).is_some() {
            changes.participants.insert(participant.clone());
        }
        let blocked: Vec<ContextId> = self
            .send_blocked
            .iter()
            .filter(|(_, (who, _))| who == participant)
            .map(|(context, _)| context.clone())
            .collect();
        for context in blocked {
            self.send_blocked.remove(&context);
            changes.contexts.insert(context);
        }
    }

    pub(super) fn conversation_security(
        &mut self,
        at: Timestamp,
        context: &ContextId,
        security: &ConversationSecurity,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        let peer = {
            let known = self
                .contexts
                .get(context)
                .ok_or_else(|| ApplyError::UnknownContext(context.clone()))?;
            let me = crate::selectors::self_participant(self);
            known
                .participants
                .iter()
                .find(|p| Some(*p) != me.as_ref())
                .or_else(|| known.participants.first())
                .cloned()
        };
        if self.security.get(context) != Some(security) {
            self.security.insert(context.clone(), security.clone());
            changes.contexts.insert(context.clone());
        }
        if let (Some(warning), Some(origin)) = (security.warning, peer) {
            let moment = Moment {
                id: MomentId::new(format!("security:{warning:?}:{context}:{}", security.epoch)),
                context_id: context.clone(),
                origin,
                at,
                kind: MomentKind::SecurityNotice(SecurityNoticePayload {
                    warning,
                    epoch: security.epoch,
                }),
                lifecycle: Lifecycle::Done,
                order: None,
            };
            self.add_service_line(at, moment, changes)?;
        }
        Ok(())
    }

    pub(super) fn send_blocked_by_identity(
        &mut self,
        context: &ContextId,
        participant: &ParticipantId,
        moment: &Option<MomentId>,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        if !self.contexts.contains_key(context) {
            return Err(ApplyError::UnknownContext(context.clone()));
        }
        self.send_blocked.insert(context.clone(), (participant.clone(), moment.clone()));
        changes.contexts.insert(context.clone());
        Ok(())
    }
}
