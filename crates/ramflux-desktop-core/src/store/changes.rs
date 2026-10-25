// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! What one applied envelope changed, and why an envelope was rejected.

use std::collections::BTreeSet;

use thiserror::Error;

use crate::ids::{
    AccountId, BackupJobId, BlockId, CallId, ContextId, DeviceId, GrantId, MomentId, NodeId,
    ObjectId, ParticipantId, StepId, SurfaceId, TaskId,
};
use crate::model::{BackupJobKind, BackupPhase, CallState, Lifecycle, StepState, TaskState};

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
    /// Account not in the store.
    #[error("unknown account {0}")]
    UnknownAccount(AccountId),
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
    /// Only approved grants can be revoked.
    #[error("grant {0} is not approved")]
    GrantNotApproved(GrantId),
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
    /// Surface not in the store.
    #[error("unknown surface {0}")]
    UnknownSurface(SurfaceId),
    /// A surface with this id already exists.
    #[error("surface {0} already exists")]
    DuplicateSurface(SurfaceId),
    /// A surface event that is not the next version.
    #[error("surface {surface}: expected version {expected}, found {found}")]
    SurfaceVersion {
        /// Surface.
        surface: SurfaceId,
        /// The version the store accepts next.
        expected: u64,
        /// The version the event carries.
        found: u64,
    },
    /// The surface was closed; it is a frozen record.
    #[error("surface {0} is closed")]
    SurfaceClosed(SurfaceId),
    /// The surface exceeded a renderer limit and shows only its fallback.
    #[error("surface {0} is over a renderer limit")]
    SurfaceOverLimit(SurfaceId),
    /// A surface without a fallback text.
    #[error("surface {0} has no fallback text")]
    MissingFallback(SurfaceId),
    /// Two blocks with one id in a surface.
    #[error("block {0} appears twice")]
    DuplicateBlock(BlockId),
    /// A block inserted past the end of the surface.
    #[error("block {block} cannot be inserted at {index}")]
    InsertOutOfRange {
        /// Block.
        block: BlockId,
        /// Requested position.
        index: usize,
    },
    /// Text was streamed into a block whose stream ended.
    #[error("the stream of block {0} has ended")]
    StreamFinished(BlockId),
    /// A task update whose version does not rise.
    #[error("task {task}: version {found} is not after {current}")]
    StaleTaskVersion {
        /// Task.
        task: TaskId,
        /// Version in the store.
        current: u64,
        /// Version of the update.
        found: u64,
    },
    /// A task update changed who the task belongs to.
    #[error("task {0} cannot change its conversation, requester or assignee")]
    TaskIdentityChanged(TaskId),
    /// Task state transition not allowed by [`TaskState::can_transition`].
    #[error("illegal task transition for {task}: {from:?} -> {to:?}")]
    IllegalTaskTransition {
        /// Task.
        task: TaskId,
        /// Current state.
        from: TaskState,
        /// Requested state.
        to: TaskState,
    },
    /// Step state transition not allowed by [`StepState::can_transition`].
    #[error("illegal step transition for {task}/{step}: {from:?} -> {to:?}")]
    IllegalStepTransition {
        /// Task.
        task: TaskId,
        /// Step.
        step: StepId,
        /// Current state.
        from: StepState,
        /// Requested state.
        to: StepState,
    },
    /// Step not in the task.
    #[error("task {task} has no step {step}")]
    UnknownStep {
        /// Task.
        task: TaskId,
        /// Requested step.
        step: StepId,
    },
    /// A call update whose version does not rise.
    #[error("call {call}: version {found} is not after {current}")]
    StaleCallVersion {
        /// Call.
        call: CallId,
        /// Version in the store.
        current: u64,
        /// Version of the update.
        found: u64,
    },
    /// A call update changed its conversation, peer, caller or direction.
    #[error("call {0} cannot change its conversation, peer, caller or direction")]
    CallIdentityChanged(CallId),
    /// Call state transition not allowed by [`CallState::can_transition`].
    #[error("illegal call transition for {call}: {from:?} -> {to:?}")]
    IllegalCallTransition {
        /// Call.
        call: CallId,
        /// Current state.
        from: CallState,
        /// Requested state.
        to: CallState,
    },
    /// A backup job update whose version does not rise.
    #[error("backup job {job}: version {found} is not after {current}")]
    StaleBackupVersion {
        /// Job.
        job: BackupJobId,
        /// Version in the store.
        current: u64,
        /// Version of the update.
        found: u64,
    },
    /// A backup job update changed its kind or file.
    #[error("backup job {0} cannot change its kind or file")]
    BackupJobChanged(BackupJobId),
    /// A job phase change not allowed by [`BackupPhase::can_transition`],
    /// or a first report in a phase the job's kind does not start in.
    #[error("illegal {kind:?} job transition for {job}: {from:?} -> {to:?}")]
    IllegalBackupTransition {
        /// Job.
        job: BackupJobId,
        /// Its kind.
        kind: BackupJobKind,
        /// Current phase (the requested one for a first report).
        from: BackupPhase,
        /// Requested phase.
        to: BackupPhase,
    },
    /// A task whose steps and state disagree (who needs the requester,
    /// where, whether a proposal started).
    #[error("task {task}: {problem}")]
    InvalidTask {
        /// Task.
        task: TaskId,
        /// What is wrong.
        problem: &'static str,
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
    /// Changed or removed nodes.
    pub nodes: BTreeSet<NodeId>,
    /// Changed devices.
    pub devices: BTreeSet<DeviceId>,
    /// Changed participants.
    pub participants: BTreeSet<ParticipantId>,
    /// Changed or removed contexts.
    pub contexts: BTreeSet<ContextId>,
    /// Changed or removed moments.
    pub moments: BTreeSet<MomentId>,
    /// Surface moments on which the authority refused an action because the
    /// surface had changed: the user reviews them again.
    pub surface_refusals: BTreeSet<MomentId>,
    /// Changed objects.
    pub objects: BTreeSet<ObjectId>,
    /// Changed grants.
    pub grants: BTreeSet<GrantId>,
    /// Changed tasks.
    pub tasks: BTreeSet<TaskId>,
    /// Changed calls.
    pub calls: BTreeSet<CallId>,
    /// Contact requests or the request setting changed.
    pub contact_requests: bool,
    /// Changed backup and restore jobs.
    pub backup_jobs: BTreeSet<BackupJobId>,
    /// Standing approvals changed.
    pub standing_approvals: bool,
    /// The audit trail grew.
    pub audit: bool,
    /// The integrations snapshot changed.
    pub integrations: bool,
    /// The pinned order changed.
    pub pin_order: bool,
    /// A collection was listed for the first time.
    pub listings: bool,
    /// Every message body was dropped (an account locked); rows that show
    /// bodies must refresh.
    pub bodies_cleared: bool,
    /// The source status, backend capabilities, daemon info or the failed
    /// effects changed.
    pub source_status: bool,
}

impl Changes {
    /// Returns true when nothing changed.
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
            && self.nodes.is_empty()
            && self.devices.is_empty()
            && self.participants.is_empty()
            && self.contexts.is_empty()
            && self.moments.is_empty()
            && self.surface_refusals.is_empty()
            && self.objects.is_empty()
            && self.grants.is_empty()
            && self.tasks.is_empty()
            && self.calls.is_empty()
            && !self.contact_requests
            && self.backup_jobs.is_empty()
            && !self.standing_approvals
            && !self.audit
            && !self.integrations
            && !self.pin_order
            && !self.listings
            && !self.bodies_cleared
            && !self.source_status
    }

    /// Adds every change of `other` to `self`.
    pub fn merge(&mut self, other: Changes) {
        let Changes {
            accounts,
            nodes,
            devices,
            participants,
            contexts,
            moments,
            surface_refusals,
            objects,
            grants,
            tasks,
            calls,
            contact_requests,
            backup_jobs,
            standing_approvals,
            audit,
            integrations,
            pin_order,
            listings,
            bodies_cleared,
            source_status,
        } = other;
        self.accounts.extend(accounts);
        self.nodes.extend(nodes);
        self.devices.extend(devices);
        self.participants.extend(participants);
        self.contexts.extend(contexts);
        self.moments.extend(moments);
        self.surface_refusals.extend(surface_refusals);
        self.objects.extend(objects);
        self.grants.extend(grants);
        self.tasks.extend(tasks);
        self.calls.extend(calls);
        self.contact_requests |= contact_requests;
        self.backup_jobs.extend(backup_jobs);
        self.standing_approvals |= standing_approvals;
        self.audit |= audit;
        self.integrations |= integrations;
        self.pin_order |= pin_order;
        self.listings |= listings;
        self.bodies_cleared |= bodies_cleared;
        self.source_status |= source_status;
    }
}
