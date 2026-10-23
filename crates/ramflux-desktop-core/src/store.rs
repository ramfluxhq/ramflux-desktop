// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The single-writer store (rule R1).
//!
//! [`Store::apply`] is the only way to change domain state. Every other
//! public method borrows immutably.

use std::collections::{BTreeMap, BTreeSet};
use std::mem;

use serde_json::Value;
use thiserror::Error;

use crate::event::{DomainEvent, EVENT_SCHEMA, EventEnvelope, SourceStatus, SurfaceOp};
use crate::ids::{
    AccountId, BlockId, ContextId, GrantId, MomentId, NodeId, ObjectId, ParticipantId, SurfaceId,
    TaskId, Timestamp,
};
use crate::model::{
    Account, AgentSurfacePayload, Context, Grant, GrantState, Lifecycle, Moment, MomentKind, Node,
    Object, Participant, Task,
};

/// Maximum number of rejections kept for inspection.
pub const MAX_REJECTIONS: usize = 64;

/// Why an envelope was not applied.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ApplyError {
    /// The envelope carries a different schema.
    #[error("schema mismatch: expected {expected}, found {found}")]
    SchemaMismatch {
        /// Supported schema.
        expected: &'static str,
        /// Received schema.
        found: String,
    },
    /// Node not in the store.
    #[error("unknown node {0}")]
    UnknownNode(NodeId),
    /// Context not in the store.
    #[error("unknown context {0}")]
    UnknownContext(ContextId),
    /// Moment not in the store.
    #[error("unknown moment {0}")]
    UnknownMoment(MomentId),
    /// Grant not in the store.
    #[error("unknown grant {0}")]
    UnknownGrant(GrantId),
    /// Task not in the store.
    #[error("unknown task {0}")]
    UnknownTask(TaskId),
    /// A moment with this id already exists.
    #[error("moment {0} already exists")]
    DuplicateMoment(MomentId),
    /// A grant with this id already exists.
    #[error("grant {0} already exists")]
    DuplicateGrant(GrantId),
    /// Lifecycle transition not allowed by [`Lifecycle::can_transition`].
    #[error("illegal lifecycle transition for {entity}: {from:?} -> {to:?}")]
    IllegalTransition {
        /// Moment, task, or task step identifier.
        entity: String,
        /// Current lifecycle.
        from: Lifecycle,
        /// Requested lifecycle.
        to: Lifecycle,
    },
    /// Grant already decided.
    #[error("grant {0} is not pending")]
    GrantNotPending(GrantId),
    /// A grant cannot be resolved back to pending.
    #[error("grant {0} cannot be resolved to pending")]
    InvalidGrantResolution(GrantId),
    /// A payload update tried to change the moment kind.
    #[error("moment {0} cannot change kind")]
    KindMismatch(MomentId),
    /// The moment does not hold the referenced surface.
    #[error("moment {moment} does not hold surface {surface}")]
    SurfaceMismatch {
        /// Moment.
        moment: MomentId,
        /// Surface.
        surface: SurfaceId,
    },
    /// Block not in the surface.
    #[error("unknown block {0}")]
    UnknownBlock(BlockId),
    /// Text was appended to a block without a string `text` field.
    #[error("block {0} has no text field")]
    NotTextBlock(BlockId),
    /// Task step index out of range.
    #[error("task {task} has no step {index}")]
    StepOutOfRange {
        /// Task.
        task: TaskId,
        /// Requested index.
        index: u32,
    },
}

/// A rejected envelope, kept for the event inspector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    /// Envelope sequence.
    pub seq: u64,
    /// Reason.
    pub error: ApplyError,
}

/// Ids changed by one applied envelope, per collection, so views can notify precisely.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// Changed accounts.
    pub accounts: BTreeSet<AccountId>,
    /// Changed nodes.
    pub nodes: BTreeSet<NodeId>,
    /// Changed participants.
    pub participants: BTreeSet<ParticipantId>,
    /// Changed or removed contexts.
    pub contexts: BTreeSet<ContextId>,
    /// Changed or removed moments.
    pub moments: BTreeSet<MomentId>,
    /// Changed objects.
    pub objects: BTreeSet<ObjectId>,
    /// Changed grants.
    pub grants: BTreeSet<GrantId>,
    /// Changed tasks.
    pub tasks: BTreeSet<TaskId>,
    /// The pinned order changed.
    pub pin_order: bool,
    /// The source status changed.
    pub source_status: bool,
}

impl Changes {
    /// Returns true when nothing changed.
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
            && self.nodes.is_empty()
            && self.participants.is_empty()
            && self.contexts.is_empty()
            && self.moments.is_empty()
            && self.objects.is_empty()
            && self.grants.is_empty()
            && self.tasks.is_empty()
            && !self.pin_order
            && !self.source_status
    }
}

/// Normalized domain state.
#[derive(Debug, Clone, Default)]
pub struct Store {
    accounts: BTreeMap<AccountId, Account>,
    nodes: BTreeMap<NodeId, Node>,
    participants: BTreeMap<ParticipantId, Participant>,
    contexts: BTreeMap<ContextId, Context>,
    moments: BTreeMap<MomentId, Moment>,
    objects: BTreeMap<ObjectId, Object>,
    grants: BTreeMap<GrantId, Grant>,
    tasks: BTreeMap<TaskId, Task>,
    pin_order: Vec<ContextId>,
    needs_me_since: BTreeMap<MomentId, Timestamp>,
    source_status: SourceStatus,
    last_seq: u64,
    rejections: Vec<Rejection>,
}

impl Store {
    /// Creates an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies one envelope.
    ///
    /// Envelopes with `seq <= last_seq` are ignored and return empty changes.
    /// A rejected envelope changes no domain state; its `seq` is still
    /// consumed (except on schema mismatch) so redelivery stays idempotent,
    /// and the error is recorded in [`Store::rejections`].
    ///
    /// # Errors
    ///
    /// Returns [`ApplyError`] when the schema differs or the event is not
    /// valid against the current state (unknown ids, illegal lifecycle
    /// transitions, kind changes).
    pub fn apply(&mut self, env: &EventEnvelope) -> Result<Changes, ApplyError> {
        if env.schema != EVENT_SCHEMA {
            let error =
                ApplyError::SchemaMismatch { expected: EVENT_SCHEMA, found: env.schema.clone() };
            self.record(env.seq, error.clone());
            return Err(error);
        }
        if env.seq <= self.last_seq {
            return Ok(Changes::default());
        }
        self.last_seq = env.seq;
        let mut changes = Changes::default();
        match self.reduce(env.at, &env.event, &mut changes) {
            Ok(()) => Ok(changes),
            Err(error) => {
                self.record(env.seq, error.clone());
                Err(error)
            }
        }
    }

    fn record(&mut self, seq: u64, error: ApplyError) {
        if self.rejections.len() == MAX_REJECTIONS {
            self.rejections.remove(0);
        }
        self.rejections.push(Rejection { seq, error });
    }

    // Each arm validates fully before its first mutation, so a rejected
    // event leaves the store untouched.
    fn reduce(
        &mut self,
        at: Timestamp,
        event: &DomainEvent,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        match event {
            DomainEvent::AccountUpdated(account) => {
                changes.accounts.insert(account.id.clone());
                self.accounts.insert(account.id.clone(), account.clone());
            }
            DomainEvent::NodeUpserted(node) => {
                changes.nodes.insert(node.id.clone());
                self.nodes.insert(node.id.clone(), node.clone());
            }
            DomainEvent::NodeHealthChanged { node_id, health } => {
                let node = self
                    .nodes
                    .get_mut(node_id)
                    .ok_or_else(|| ApplyError::UnknownNode(node_id.clone()))?;
                node.health = *health;
                changes.nodes.insert(node_id.clone());
            }
            DomainEvent::ParticipantUpserted(participant) => {
                changes.participants.insert(participant.id.clone());
                self.participants.insert(participant.id.clone(), participant.clone());
            }
            DomainEvent::ContextUpserted(context) => self.upsert_context(context, changes),
            DomainEvent::ContextRemoved { context_id } => {
                self.remove_context(context_id, changes)?;
            }
            DomainEvent::MomentAdded(moment) => self.add_moment(at, moment, changes)?,
            DomainEvent::MomentUpdated { moment_id, lifecycle, kind } => {
                self.update_moment(at, moment_id, *lifecycle, kind.as_ref(), changes)?;
            }
            DomainEvent::SurfacePatched { moment_id, surface_id, ops } => {
                self.patch_surface(moment_id, surface_id, ops, changes)?;
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
                let grant = self
                    .grants
                    .get_mut(grant_id)
                    .ok_or_else(|| ApplyError::UnknownGrant(grant_id.clone()))?;
                if *state == GrantState::Pending {
                    return Err(ApplyError::InvalidGrantResolution(grant_id.clone()));
                }
                if grant.state != GrantState::Pending {
                    return Err(ApplyError::GrantNotPending(grant_id.clone()));
                }
                grant.state = *state;
                changes.grants.insert(grant_id.clone());
            }
            DomainEvent::TaskUpserted(task) => self.upsert_task(task, changes)?,
            DomainEvent::TaskStepChanged { task_id, index, lifecycle } => {
                let task = self
                    .tasks
                    .get_mut(task_id)
                    .ok_or_else(|| ApplyError::UnknownTask(task_id.clone()))?;
                let step =
                    usize::try_from(*index).ok().and_then(|i| task.steps.get_mut(i)).ok_or_else(
                        || ApplyError::StepOutOfRange { task: task_id.clone(), index: *index },
                    )?;
                check_transition(
                    || format!("task {task_id} step {index}"),
                    step.lifecycle,
                    *lifecycle,
                )?;
                step.lifecycle = *lifecycle;
                changes.tasks.insert(task_id.clone());
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

    fn upsert_context(&mut self, context: &Context, changes: &mut Changes) {
        let in_order = self.pin_order.contains(&context.id);
        if context.pinned && !in_order {
            self.pin_order.push(context.id.clone());
            changes.pin_order = true;
        } else if !context.pinned && in_order {
            self.pin_order.retain(|id| id != &context.id);
            changes.pin_order = true;
        }
        changes.contexts.insert(context.id.clone());
        self.contexts.insert(context.id.clone(), context.clone());
    }

    fn remove_context(
        &mut self,
        context_id: &ContextId,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        if self.contexts.remove(context_id).is_none() {
            return Err(ApplyError::UnknownContext(context_id.clone()));
        }
        changes.contexts.insert(context_id.clone());
        if self.pin_order.contains(context_id) {
            self.pin_order.retain(|id| id != context_id);
            changes.pin_order = true;
        }
        let removed: Vec<MomentId> = self
            .moments
            .values()
            .filter(|m| &m.context_id == context_id)
            .map(|m| m.id.clone())
            .collect();
        for id in removed {
            self.moments.remove(&id);
            self.needs_me_since.remove(&id);
            changes.moments.insert(id);
        }
        Ok(())
    }

    fn add_moment(
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
        if moment.lifecycle == Lifecycle::NeedsMe {
            self.needs_me_since.insert(moment.id.clone(), at);
        }
        changes.moments.insert(moment.id.clone());
        self.moments.insert(moment.id.clone(), moment.clone());
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
        let next = lifecycle.unwrap_or(moment.lifecycle);
        if next != moment.lifecycle {
            check_transition(|| format!("moment {moment_id}"), moment.lifecycle, next)?;
        }
        if let Some(kind) = kind
            && mem::discriminant(kind) != mem::discriminant(&moment.kind)
        {
            return Err(ApplyError::KindMismatch(moment_id.clone()));
        }
        if next != moment.lifecycle {
            if next == Lifecycle::NeedsMe {
                self.needs_me_since.insert(moment_id.clone(), at);
            } else {
                self.needs_me_since.remove(moment_id);
            }
            moment.lifecycle = next;
        }
        if let Some(kind) = kind {
            moment.kind = kind.clone();
        }
        changes.moments.insert(moment_id.clone());
        Ok(())
    }

    fn patch_surface(
        &mut self,
        moment_id: &MomentId,
        surface_id: &SurfaceId,
        ops: &[SurfaceOp],
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        let moment = self
            .moments
            .get_mut(moment_id)
            .ok_or_else(|| ApplyError::UnknownMoment(moment_id.clone()))?;
        let MomentKind::AgentSurface(surface) = &mut moment.kind else {
            return Err(ApplyError::SurfaceMismatch {
                moment: moment_id.clone(),
                surface: surface_id.clone(),
            });
        };
        if &surface.surface_id != surface_id {
            return Err(ApplyError::SurfaceMismatch {
                moment: moment_id.clone(),
                surface: surface_id.clone(),
            });
        }
        let mut patched = surface.clone();
        for op in ops {
            apply_surface_op(&mut patched, op)?;
        }
        *surface = patched;
        changes.moments.insert(moment_id.clone());
        Ok(())
    }

    fn upsert_task(&mut self, task: &Task, changes: &mut Changes) -> Result<(), ApplyError> {
        if !self.contexts.contains_key(&task.context_id) {
            return Err(ApplyError::UnknownContext(task.context_id.clone()));
        }
        if let Some(current) = self.tasks.get(&task.id)
            && current.lifecycle != task.lifecycle
        {
            check_transition(|| format!("task {}", task.id), current.lifecycle, task.lifecycle)?;
        }
        changes.tasks.insert(task.id.clone());
        self.tasks.insert(task.id.clone(), task.clone());
        Ok(())
    }

    /// Last applied sequence number (0 before any envelope).
    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    /// Current source status.
    pub fn source_status(&self) -> SourceStatus {
        self.source_status
    }

    /// Most recent rejections, oldest first, at most [`MAX_REJECTIONS`].
    pub fn rejections(&self) -> &[Rejection] {
        &self.rejections
    }

    /// Pinned contexts in the order they were pinned.
    pub fn pin_order(&self) -> &[ContextId] {
        &self.pin_order
    }

    /// When a `NeedsMe` moment entered that state.
    pub fn needs_me_since(&self, id: &MomentId) -> Option<Timestamp> {
        self.needs_me_since.get(id).copied()
    }

    /// Account by id.
    pub fn account(&self, id: &AccountId) -> Option<&Account> {
        self.accounts.get(id)
    }

    /// All accounts ordered by id.
    pub fn accounts(&self) -> impl Iterator<Item = &Account> {
        self.accounts.values()
    }

    /// Node by id.
    pub fn node(&self, id: &NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// All nodes ordered by id.
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    /// Participant by id.
    pub fn participant(&self, id: &ParticipantId) -> Option<&Participant> {
        self.participants.get(id)
    }

    /// All participants ordered by id.
    pub fn participants(&self) -> impl Iterator<Item = &Participant> {
        self.participants.values()
    }

    /// Context by id.
    pub fn context(&self, id: &ContextId) -> Option<&Context> {
        self.contexts.get(id)
    }

    /// All contexts ordered by id.
    pub fn contexts(&self) -> impl Iterator<Item = &Context> {
        self.contexts.values()
    }

    /// Moment by id.
    pub fn moment(&self, id: &MomentId) -> Option<&Moment> {
        self.moments.get(id)
    }

    /// All moments ordered by id.
    pub fn moments(&self) -> impl Iterator<Item = &Moment> {
        self.moments.values()
    }

    /// Object by id.
    pub fn object(&self, id: &ObjectId) -> Option<&Object> {
        self.objects.get(id)
    }

    /// All objects ordered by id.
    pub fn objects(&self) -> impl Iterator<Item = &Object> {
        self.objects.values()
    }

    /// Grant by id.
    pub fn grant(&self, id: &GrantId) -> Option<&Grant> {
        self.grants.get(id)
    }

    /// All grants ordered by id.
    pub fn grants(&self) -> impl Iterator<Item = &Grant> {
        self.grants.values()
    }

    /// Task by id.
    pub fn task(&self, id: &TaskId) -> Option<&Task> {
        self.tasks.get(id)
    }

    /// All tasks ordered by id.
    pub fn tasks(&self) -> impl Iterator<Item = &Task> {
        self.tasks.values()
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

fn apply_surface_op(surface: &mut AgentSurfacePayload, op: &SurfaceOp) -> Result<(), ApplyError> {
    match op {
        SurfaceOp::AppendBlocks(blocks) => surface.blocks.extend(blocks.iter().cloned()),
        SurfaceOp::ReplaceBlock(block) => {
            let slot = surface
                .blocks
                .iter_mut()
                .find(|b| b.id == block.id)
                .ok_or_else(|| ApplyError::UnknownBlock(block.id.clone()))?;
            *slot = block.clone();
        }
        SurfaceOp::ReplaceBlocks(blocks) => surface.blocks.clone_from(blocks),
        SurfaceOp::AppendText { block_id, text } => {
            let block = surface
                .blocks
                .iter_mut()
                .find(|b| &b.id == block_id)
                .ok_or_else(|| ApplyError::UnknownBlock(block_id.clone()))?;
            match block.data.get_mut("text") {
                Some(Value::String(current)) => current.push_str(text),
                _ => return Err(ApplyError::NotTextBlock(block_id.clone())),
            }
        }
        SurfaceOp::SetActions(actions) => surface.actions.clone_from(actions),
        SurfaceOp::SetStreaming(streaming) => surface.streaming = *streaming,
    }
    Ok(())
}
