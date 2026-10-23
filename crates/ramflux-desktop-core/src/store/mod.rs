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
//! - `surfaces`: surface content hashes, compare-and-swap patches, conflicts.
//! - [`bodies`]: the bounded message-body cache.
//! - `index`: secondary indexes the reducer maintains (timelines, needs-me,
//!   in progress, members, grant and task presenters).

pub mod bodies;
mod changes;
mod index;
mod reducer;
mod surface_ops;
mod surfaces;

use std::collections::{BTreeMap, BTreeSet};

use crate::event::{EVENT_SCHEMA, EventEnvelope, SourceStatus};
use crate::ids::{
    AccountId, ContextId, DeviceId, GrantId, MomentId, NodeId, ObjectId, ParticipantId,
    StandingApprovalId, TaskId, Timestamp,
};
use crate::model::{
    Account, AuditEntry, Capabilities, Context, DaemonInfo, Device, EffectFailure, Grant,
    Integrations, Listing, Moment, Node, Object, Participant, StandingApproval, Task,
};

pub use bodies::{BodyCache, MAX_BODIES};
pub use changes::{ApplyError, Changes, Rejection};

use index::Indexes;

/// Maximum number of rejections kept for inspection.
pub const MAX_REJECTIONS: usize = 64;

/// Maximum number of audit records kept in memory.
pub const MAX_AUDIT: usize = 512;

/// Maximum number of failed effects kept for display.
pub const MAX_FAILURES: usize = 32;

/// Normalized domain state.
///
/// Moment rows hold metadata; message text lives in a bounded body cache
/// ([`Store::body`]). Secondary indexes keep every selector proportional to
/// its result.
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
    capabilities: Capabilities,
    daemon: Option<DaemonInfo>,
    failures: Vec<EffectFailure>,
    failures_total: u64,
    pin_order: Vec<ContextId>,
    listed: BTreeSet<Listing>,
    needs_me_since: BTreeMap<MomentId, Timestamp>,
    surface_hashes: BTreeMap<MomentId, crate::conformance::SurfaceHash>,
    surface_conflicts: BTreeSet<MomentId>,
    source_status: SourceStatus,
    last_seq: u64,
    rejections: Vec<Rejection>,
    index: Indexes,
    bodies: BodyCache,
}

impl Store {
    /// Creates an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty store keeping at most `capacity` message bodies.
    pub fn with_body_capacity(capacity: usize) -> Self {
        Self { bodies: BodyCache::with_capacity(capacity), ..Self::default() }
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

    /// Whether the source has listed `listing` (so an empty collection is
    /// empty, not still loading).
    pub fn is_listed(&self, listing: Listing) -> bool {
        self.listed.contains(&listing)
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

    /// What the backend cannot do, with reasons.
    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    /// Why `method` is unavailable on this backend, if it is.
    pub fn unsupported_reason(&self, method: &str) -> Option<&str> {
        self.capabilities.unsupported_reason(method)
    }

    /// What the local daemon last reported about itself.
    pub fn daemon(&self) -> Option<&DaemonInfo> {
        self.daemon.as_ref()
    }

    /// Effects that failed, oldest first, at most [`MAX_FAILURES`].
    pub fn failures(&self) -> &[EffectFailure] {
        &self.failures
    }

    /// How many effects failed since the store was created (keeps counting
    /// past [`MAX_FAILURES`]), so a view can tell which failures are new.
    pub fn failures_total(&self) -> u64 {
        self.failures_total
    }

    /// The failures recorded after the count was `since`, oldest first
    /// (those still kept).
    pub fn failures_since(&self, since: u64) -> &[EffectFailure] {
        let new = usize::try_from(self.failures_total.saturating_sub(since)).unwrap_or(usize::MAX);
        &self.failures[self.failures.len().saturating_sub(new)..]
    }

    /// Task by id.
    pub fn task(&self, id: &TaskId) -> Option<&Task> {
        self.tasks.get(id)
    }

    /// The text of a message moment, while its body is in memory. `None`
    /// for other kinds and for bodies evicted or dropped on lock; the view
    /// then asks the source to deliver it again.
    pub fn body(&self, id: &MomentId) -> Option<&str> {
        self.bodies.get(id).map(|text| &**text)
    }

    /// The message-body cache (size and capacity).
    pub fn bodies(&self) -> &BodyCache {
        &self.bodies
    }

    /// Moments of one context ordered by `(at, id)`; O(k).
    pub fn context_moments(&self, id: &ContextId) -> impl DoubleEndedIterator<Item = &Moment> {
        self.index
            .timeline
            .get(id)
            .into_iter()
            .flatten()
            .filter_map(|(_, moment)| self.moments.get(moment))
    }

    /// Number of moments in one context.
    pub fn context_moment_count(&self, id: &ContextId) -> usize {
        self.index.timeline.get(id).map_or(0, std::collections::BTreeSet::len)
    }

    /// `NeedsMe` moments with the time they entered it, oldest first (then
    /// by id); O(k).
    pub fn needs_me(&self) -> impl Iterator<Item = (Timestamp, &Moment)> {
        self.index
            .needs_me
            .iter()
            .filter_map(|(since, id)| self.moments.get(id).map(|m| (*since, m)))
    }

    /// `NeedsMe` moments of one context, by id.
    pub fn needs_me_in(&self, context: &ContextId) -> impl Iterator<Item = &Moment> {
        self.index
            .needs_me_by_context
            .get(context)
            .into_iter()
            .flatten()
            .filter_map(|id| self.moments.get(id))
    }

    /// `InProgress` moments, by id.
    pub fn in_progress(&self) -> impl Iterator<Item = &Moment> {
        self.index.in_progress.iter().filter_map(|id| self.moments.get(id))
    }

    /// Contexts `participant` is a member of, by id.
    pub fn contexts_of(&self, participant: &ParticipantId) -> impl Iterator<Item = &Context> {
        self.index
            .contexts_by_participant
            .get(participant)
            .into_iter()
            .flatten()
            .filter_map(|id| self.contexts.get(id))
    }

    /// Moments presenting `grant` (interventions, approval blocks), by id.
    pub fn grant_presenters(&self, grant: &GrantId) -> impl Iterator<Item = &Moment> {
        self.index
            .grant_presenters
            .get(grant)
            .into_iter()
            .flatten()
            .filter_map(|id| self.moments.get(id))
    }

    /// Task progress moments presenting `task`, by id.
    pub fn task_presenters(&self, task: &TaskId) -> impl Iterator<Item = &Moment> {
        self.index
            .task_presenters
            .get(task)
            .into_iter()
            .flatten()
            .filter_map(|id| self.moments.get(id))
    }

    /// Tasks with a step waiting on `grant`, by id.
    pub fn step_owners(&self, grant: &GrantId) -> impl Iterator<Item = &Task> {
        self.index.step_owners.get(grant).into_iter().flatten().filter_map(|id| self.tasks.get(id))
    }

    /// All tasks ordered by id.
    pub fn tasks(&self) -> impl Iterator<Item = &Task> {
        self.tasks.values()
    }
}
