// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The reducer: one arm per domain event.
//!
//! Each arm validates fully before its first mutation, so a rejected event
//! leaves the store untouched. Every mutation of moments, contexts and tasks
//! updates the secondary indexes in the same arm, and message text moves
//! into the body cache.

use std::mem;

use super::bodies::BodyCache;
use super::changes::{ApplyError, Changes};
use super::tasks::{StateChange, StepChange};
use super::{MAX_AUDIT, Store};
use crate::event::DomainEvent;
use crate::ids::{AccountId, ContextId, GrantId, MomentId, NodeId, Timestamp};
use crate::model::{
    Context, Disappearing, GrantState, Lifecycle, LockState, Moment, MomentKind, NodeHealth,
    NodeProbe,
};

impl Store {
    pub(super) fn reduce(
        &mut self,
        at: Timestamp,
        event: &DomainEvent,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        match event {
            DomainEvent::AccountUpdated(account) => {
                changes.accounts.insert(account.id.clone());
                let was = self.accounts.insert(account.id.clone(), account.clone());
                let was_unlocked = was.is_some_and(|a| a.lock_state == LockState::Unlocked);
                if was_unlocked && account.lock_state == LockState::Locked {
                    self.drop_bodies(changes);
                }
            }
            DomainEvent::AccountLockChanged { account_id, lock_state } => {
                self.set_lock(account_id, *lock_state, changes)?;
            }
            DomainEvent::NodeUpserted(node) => {
                changes.nodes.insert(node.id.clone());
                self.nodes.insert(node.id.clone(), node.clone());
            }
            DomainEvent::NodeHealthChanged { node_id, health } => {
                self.node_mut(node_id)?.health = *health;
                changes.nodes.insert(node_id.clone());
            }
            DomainEvent::NodeSettingsChanged { node_id, enabled, is_default } => {
                self.node_settings(node_id, *enabled, *is_default, changes)?;
            }
            DomainEvent::NodeProbed { node_id, probe } => {
                let node = self.node_mut(node_id)?;
                node.health = probe_health(probe);
                node.probe = Some(probe.clone());
                changes.nodes.insert(node_id.clone());
            }
            DomainEvent::NodeRemoved { node_id } => {
                if self.nodes.remove(node_id).is_none() {
                    return Err(ApplyError::UnknownNode(node_id.clone()));
                }
                changes.nodes.insert(node_id.clone());
            }
            DomainEvent::DeviceUpserted(device) => {
                changes.devices.insert(device.id.clone());
                self.devices.insert(device.id.clone(), device.clone());
            }
            DomainEvent::ParticipantUpserted(participant) => {
                changes.participants.insert(participant.id.clone());
                self.participants.insert(participant.id.clone(), participant.clone());
            }
            DomainEvent::ContextUpserted(context) => self.upsert_context(context, changes),
            DomainEvent::ContextSettingsChanged { context_id, pinned, muted, disappearing } => {
                self.context_settings(context_id, *pinned, *muted, *disappearing, changes)?;
            }
            DomainEvent::ContextRemoved { context_id } => {
                self.remove_context(context_id, changes)?;
            }
            DomainEvent::MomentAdded(moment) => self.add_moment(at, moment, changes)?,
            DomainEvent::MomentUpdated { moment_id, lifecycle, kind } => {
                self.update_moment(at, moment_id, *lifecycle, kind.as_ref(), changes)?;
            }
            DomainEvent::Surface(event) => self.surface_event(at, event, changes)?,
            DomainEvent::SurfacePatched { moment_id, surface_id, ops, base_hash } => {
                self.patch_surface(moment_id, surface_id, ops, base_hash.as_ref(), changes)?;
            }
            DomainEvent::ObjectUpserted(object) => {
                changes.objects.insert(object.id.clone());
                self.objects.insert(object.id.clone(), object.clone());
            }
            DomainEvent::GrantRequested(grant) => {
                if self.grants.contains_key(&grant.id) {
                    return Err(ApplyError::DuplicateGrant(grant.id.clone()));
                }
                let mut grant = grant.clone();
                grant.state = GrantState::Pending;
                changes.grants.insert(grant.id.clone());
                self.grants.insert(grant.id.clone(), grant);
            }
            DomainEvent::GrantResolved { grant_id, state } => {
                self.resolve_grant(grant_id, *state, changes)?;
            }
            DomainEvent::GrantFailed { grant_id, failure } => {
                let grant = self
                    .grants
                    .get_mut(grant_id)
                    .ok_or_else(|| ApplyError::UnknownGrant(grant_id.clone()))?;
                grant.failure.get_or_insert(*failure);
                changes.grants.insert(grant_id.clone());
            }
            DomainEvent::GrantRevoked { grant_id } => {
                let grant = self
                    .grants
                    .get_mut(grant_id)
                    .ok_or_else(|| ApplyError::UnknownGrant(grant_id.clone()))?;
                if grant.state != GrantState::Approved {
                    return Err(ApplyError::GrantNotApproved(grant_id.clone()));
                }
                grant.state = GrantState::Revoked;
                changes.grants.insert(grant_id.clone());
            }
            DomainEvent::StandingApprovalUpserted(approval) => {
                changes.standing_approvals = true;
                self.standing_approvals.insert(approval.id.clone(), approval.clone());
            }
            DomainEvent::AuditAppended(entry) => {
                if self.audit.len() == MAX_AUDIT {
                    self.audit.remove(0);
                }
                self.audit.push(entry.clone());
                changes.audit = true;
            }
            DomainEvent::CapabilitiesUpdated(capabilities) => {
                self.capabilities = capabilities.clone();
                changes.source_status = true;
            }
            DomainEvent::DaemonInfoUpdated(info) => {
                self.daemon = Some(info.clone());
                changes.source_status = true;
            }
            DomainEvent::SyncStatusChanged(status) => {
                if self.sync.as_ref() != Some(status) {
                    self.sync = Some(status.clone());
                    changes.source_status = true;
                }
            }
            DomainEvent::EffectFailed(failure) => {
                if self.failures.len() == super::MAX_FAILURES {
                    self.failures.remove(0);
                }
                self.failures.push(failure.clone());
                self.failures_total += 1;
                changes.source_status = true;
            }
            DomainEvent::OwnContactCardUpdated(card) => {
                if self.own_card.as_ref() != Some(card) {
                    self.own_card = Some(card.clone());
                    changes.source_status = true;
                }
            }
            DomainEvent::ListingsLoaded(listings) => {
                for listing in listings {
                    changes.listings |= self.listed.insert(*listing);
                }
            }
            DomainEvent::IntegrationsUpdated(integrations) => {
                self.integrations = integrations.clone();
                changes.integrations = true;
            }
            DomainEvent::TaskUpserted(task) => self.upsert_task(at, task, changes)?,
            DomainEvent::TaskStepChanged { task_id, version, step_id, state, needs_me } => {
                let step = StepChange {
                    task_id,
                    version: *version,
                    step_id,
                    state: *state,
                    needs_me: needs_me.as_ref(),
                };
                self.change_step(at, step, changes)?;
            }
            DomainEvent::TaskStateChanged { task_id, version, state, result } => {
                let update = StateChange {
                    task_id,
                    version: *version,
                    state: *state,
                    result: result.as_ref(),
                };
                self.change_task_state(at, update, changes)?;
            }
            DomainEvent::TaskSnapshot(task) => self.snapshot_task(at, task, changes)?,
            DomainEvent::TaskAnswered { task_id, answer } => {
                self.answer_task(task_id, answer, changes)?;
            }
            DomainEvent::SourceStatusChanged(status) => {
                if self.source_status != *status {
                    self.source_status = *status;
                    changes.source_status = true;
                }
            }
        }
        Ok(())
    }

    fn node_mut(&mut self, node_id: &NodeId) -> Result<&mut crate::model::Node, ApplyError> {
        self.nodes.get_mut(node_id).ok_or_else(|| ApplyError::UnknownNode(node_id.clone()))
    }

    fn set_lock(
        &mut self,
        account_id: &AccountId,
        lock_state: LockState,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        let account = self
            .accounts
            .get_mut(account_id)
            .ok_or_else(|| ApplyError::UnknownAccount(account_id.clone()))?;
        let locking = account.lock_state == LockState::Unlocked && lock_state == LockState::Locked;
        account.lock_state = lock_state;
        changes.accounts.insert(account_id.clone());
        if locking {
            self.drop_bodies(changes);
        }
        Ok(())
    }

    /// Message bodies never outlive an unlocked account in memory.
    fn drop_bodies(&mut self, changes: &mut Changes) {
        if self.bodies.clear() {
            changes.bodies_cleared = true;
        }
    }

    fn node_settings(
        &mut self,
        node_id: &NodeId,
        enabled: Option<bool>,
        is_default: Option<bool>,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        if !self.nodes.contains_key(node_id) {
            return Err(ApplyError::UnknownNode(node_id.clone()));
        }
        if is_default == Some(true) {
            for node in self.nodes.values_mut().filter(|n| n.is_default && &n.id != node_id) {
                node.is_default = false;
                changes.nodes.insert(node.id.clone());
            }
        }
        let node = self.node_mut(node_id)?;
        if let Some(enabled) = enabled {
            node.enabled = enabled;
        }
        if let Some(is_default) = is_default {
            node.is_default = is_default;
        }
        changes.nodes.insert(node_id.clone());
        Ok(())
    }

    fn resolve_grant(
        &mut self,
        grant_id: &GrantId,
        state: GrantState,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        let grant = self
            .grants
            .get_mut(grant_id)
            .ok_or_else(|| ApplyError::UnknownGrant(grant_id.clone()))?;
        if matches!(state, GrantState::Pending | GrantState::Revoked) {
            return Err(ApplyError::InvalidGrantResolution(grant_id.clone()));
        }
        if grant.state != GrantState::Pending {
            return Err(ApplyError::GrantNotPending(grant_id.clone()));
        }
        grant.state = state;
        changes.grants.insert(grant_id.clone());
        Ok(())
    }

    fn upsert_context(&mut self, context: &Context, changes: &mut Changes) {
        self.sync_pin_order(&context.id, context.pinned, changes);
        changes.contexts.insert(context.id.clone());
        let old = self.contexts.insert(context.id.clone(), context.clone());
        let old_members = old.map(|c| c.participants).unwrap_or_default();
        if old_members != context.participants {
            self.index.change_members(&context.id, &old_members, &context.participants);
        }
    }

    fn sync_pin_order(&mut self, id: &ContextId, pinned: bool, changes: &mut Changes) {
        let in_order = self.pin_order.contains(id);
        if pinned && !in_order {
            self.pin_order.push(id.clone());
            changes.pin_order = true;
        } else if !pinned && in_order {
            self.pin_order.retain(|other| other != id);
            changes.pin_order = true;
        }
    }

    fn context_settings(
        &mut self,
        context_id: &ContextId,
        pinned: Option<bool>,
        muted: Option<bool>,
        disappearing: Option<Disappearing>,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        let context = self
            .contexts
            .get_mut(context_id)
            .ok_or_else(|| ApplyError::UnknownContext(context_id.clone()))?;
        if let Some(muted) = muted {
            context.muted = muted;
        }
        if let Some(disappearing) = disappearing {
            context.disappearing_secs = match disappearing {
                Disappearing::Off => None,
                Disappearing::After(secs) => Some(secs),
            };
        }
        if let Some(pinned) = pinned {
            context.pinned = pinned;
            self.sync_pin_order(context_id, pinned, changes);
        }
        changes.contexts.insert(context_id.clone());
        Ok(())
    }

    fn remove_context(
        &mut self,
        context_id: &ContextId,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        let Some(context) = self.contexts.remove(context_id) else {
            return Err(ApplyError::UnknownContext(context_id.clone()));
        };
        changes.contexts.insert(context_id.clone());
        if self.pin_order.contains(context_id) {
            self.pin_order.retain(|id| id != context_id);
            changes.pin_order = true;
        }
        let removed: Vec<MomentId> = self
            .index
            .timeline
            .get(context_id)
            .into_iter()
            .flatten()
            .map(|(_, id)| id.clone())
            .collect();
        for id in removed {
            if let Some(moment) = self.moments.remove(&id) {
                let since = self.needs_me_since.remove(&id);
                self.index.remove_moment(&moment, since);
            }
            self.bodies.remove(&id);
            self.forget_surface(&id);
            changes.moments.insert(id);
        }
        self.index.remove_context(context_id, &context.participants);
        Ok(())
    }

    pub(super) fn add_moment(
        &mut self,
        at: Timestamp,
        moment: &Moment,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        if self.moments.contains_key(&moment.id) {
            return Err(ApplyError::DuplicateMoment(moment.id.clone()));
        }
        if !self.contexts.contains_key(&moment.context_id) {
            return Err(ApplyError::UnknownContext(moment.context_id.clone()));
        }
        if let MomentKind::AgentSurface(surface) = &moment.kind
            && self.surface_ids.contains_key(&surface.surface_id)
        {
            return Err(ApplyError::DuplicateSurface(surface.surface_id.clone()));
        }
        let mut stored = moment.clone();
        // A task moment follows its task.
        if let Some(lifecycle) = self.task_moment_lifecycle(&stored.kind) {
            stored.lifecycle = lifecycle;
        }
        let since = (stored.lifecycle == Lifecycle::NeedsMe).then_some(at);
        if let Some(since) = since {
            self.needs_me_since.insert(moment.id.clone(), since);
        }
        keep_body(&mut self.bodies, &stored.id, &mut stored.kind);
        self.track_surface(&moment.id, &mut stored.kind);
        self.index.add_moment(&stored, since);
        changes.moments.insert(moment.id.clone());
        self.moments.insert(moment.id.clone(), stored);
        Ok(())
    }

    fn update_moment(
        &mut self,
        at: Timestamp,
        moment_id: &MomentId,
        lifecycle: Option<Lifecycle>,
        kind: Option<&MomentKind>,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        let moment = self
            .moments
            .get_mut(moment_id)
            .ok_or_else(|| ApplyError::UnknownMoment(moment_id.clone()))?;
        // A task moment follows its task, not lifecycle updates.
        let follows_task = matches!(&moment.kind, MomentKind::TaskProgress(p)
            if self.tasks.contains_key(&p.task_id));
        let next =
            if follows_task { moment.lifecycle } else { lifecycle.unwrap_or(moment.lifecycle) };
        if next != moment.lifecycle {
            check_transition(|| format!("moment {moment_id}"), moment.lifecycle, next)?;
        }
        if let Some(kind) = kind
            && mem::discriminant(kind) != mem::discriminant(&moment.kind)
        {
            return Err(ApplyError::KindMismatch(moment_id.clone()));
        }
        if next != moment.lifecycle {
            let old_since = if next == Lifecycle::NeedsMe {
                self.needs_me_since.insert(moment_id.clone(), at)
            } else {
                self.needs_me_since.remove(moment_id)
            };
            let new_since = (next == Lifecycle::NeedsMe).then_some(at);
            self.index.change_lifecycle(moment, (moment.lifecycle, old_since), (next, new_since));
            moment.lifecycle = next;
        }
        if let Some(kind) = kind {
            let mut kind = kind.clone();
            keep_body(&mut self.bodies, moment_id, &mut kind);
            self.track_surface(moment_id, &mut kind);
            let moment = self
                .moments
                .get_mut(moment_id)
                .ok_or_else(|| ApplyError::UnknownMoment(moment_id.clone()))?;
            self.index.change_kind(moment_id, &moment.kind, &kind);
            moment.kind = kind;
        }
        changes.moments.insert(moment_id.clone());
        Ok(())
    }
}

/// Moves a message's text into the body cache; the stored row keeps an
/// empty text (metadata only).
fn keep_body(bodies: &mut BodyCache, id: &MomentId, kind: &mut MomentKind) {
    if let MomentKind::Message(payload) = kind {
        bodies.insert(id, mem::take(&mut payload.text));
    }
}

/// Health implied by a probe: both services up is healthy, one is degraded.
fn probe_health(probe: &NodeProbe) -> NodeHealth {
    match (probe.gateway.ok, probe.relay.ok) {
        (true, true) => NodeHealth::Healthy,
        (true, false) | (false, true) => NodeHealth::Degraded,
        (false, false) => NodeHealth::Unreachable,
    }
}

fn check_transition(
    entity: impl FnOnce() -> String,
    from: Lifecycle,
    to: Lifecycle,
) -> Result<(), ApplyError> {
    if Lifecycle::can_transition(from, to) {
        Ok(())
    } else {
        Err(ApplyError::IllegalTransition { entity: entity(), from, to })
    }
}
