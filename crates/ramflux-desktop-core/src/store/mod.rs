// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The single-writer store (rule R1).
//!
//! [`Store::apply`] is the only way to change domain state. Every other
//! public method borrows immutably.
//!
//! - [`changes`]: [`Changes`], [`ApplyError`] and [`Rejection`].
//! - `reducer`: one validated arm per domain event.
//! - `surface_ops`: agent surface patch operations.

mod changes;
mod reducer;
mod surface_ops;

use std::collections::BTreeMap;

use crate::event::{EVENT_SCHEMA, EventEnvelope, SourceStatus};
use crate::ids::{
    AccountId, ContextId, DeviceId, GrantId, MomentId, NodeId, ObjectId, ParticipantId,
    StandingApprovalId, TaskId, Timestamp,
};
use crate::model::{
    Account, AuditEntry, Context, Device, Grant, Integrations, Moment, Node, Object, Participant,
    StandingApproval, Task,
};

pub use changes::{ApplyError, Changes, Rejection};

/// Maximum number of rejections kept for inspection.
pub const MAX_REJECTIONS: usize = 64;

/// Maximum number of audit records kept in memory.
pub const MAX_AUDIT: usize = 512;

/// Normalized domain state.
#[derive(Debug, Clone, Default)]
pub struct Store {
    accounts: BTreeMap<AccountId, Account>,
    nodes: BTreeMap<NodeId, Node>,
    devices: BTreeMap<DeviceId, Device>,
    participants: BTreeMap<ParticipantId, Participant>,
    contexts: BTreeMap<ContextId, Context>,
    moments: BTreeMap<MomentId, Moment>,
    objects: BTreeMap<ObjectId, Object>,
    grants: BTreeMap<GrantId, Grant>,
    standing_approvals: BTreeMap<StandingApprovalId, StandingApproval>,
    tasks: BTreeMap<TaskId, Task>,
    audit: Vec<AuditEntry>,
    integrations: Integrations,
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

    /// Device by id.
    pub fn device(&self, id: &DeviceId) -> Option<&Device> {
        self.devices.get(id)
    }

    /// All devices ordered by id.
    pub fn devices(&self) -> impl Iterator<Item = &Device> {
        self.devices.values()
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

    /// Standing approval by id.
    pub fn standing_approval(&self, id: &StandingApprovalId) -> Option<&StandingApproval> {
        self.standing_approvals.get(id)
    }

    /// All standing approvals ordered by id.
    pub fn standing_approvals(&self) -> impl Iterator<Item = &StandingApproval> {
        self.standing_approvals.values()
    }

    /// Audit records, oldest first, at most [`MAX_AUDIT`].
    pub fn audit(&self) -> &[AuditEntry] {
        &self.audit
    }

    /// The read-only integrations snapshot.
    pub fn integrations(&self) -> &Integrations {
        &self.integrations
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
