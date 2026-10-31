// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Domain events: facts flowing from a source into the store.

use serde::{Deserialize, Serialize};

use crate::conformance::SurfaceHash;
use crate::ids::{
    AccountId, BlockId, ContextId, GrantId, MomentId, NodeId, StepId, SurfaceId, TaskId, Timestamp,
};
use crate::model::{
    Account, AgentExport, AuditEntry, BackupJob, Block, Call, Capabilities, Ceremony, ContactCard,
    ContactRequest, Context, DaemonInfo, Device, Disappearing, EffectFailure, Grant, GrantFailure,
    GrantState, Integrations, Lifecycle, Listing, LockState, Moment, MomentKind, NeedsMe, Node,
    NodeHealth, NodeProbe, Object, Participant, RecoveryCeremony, RequestFloors, RequestPolicy,
    StandingApproval, StepState, SurfaceAction, SyncStatus, Task, TaskAnswer, TaskState,
};

pub use crate::surface_event::{SurfaceEvent, SurfacePatchOp, SurfaceSnapshot};

/// Schema tag carried by every envelope. The store rejects any other value.
pub const EVENT_SCHEMA: &str = "ramflux.desktop.event.v1";

/// A sequenced, timestamped domain event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    /// Must equal [`EVENT_SCHEMA`].
    pub schema: String,
    /// Monotonic sequence assigned by the source; starts at 1.
    pub seq: u64,
    /// When the source observed the fact.
    pub at: Timestamp,
    /// The fact.
    pub event: DomainEvent,
}

impl EventEnvelope {
    /// Builds an envelope with the current schema.
    pub fn new(seq: u64, at: Timestamp, event: DomainEvent) -> Self {
        Self { schema: EVENT_SCHEMA.to_owned(), seq, at, event }
    }
}

/// Connection state of the source as a whole.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceStatus {
    /// Live.
    Connected,
    /// Not reachable.
    #[default]
    Disconnected,
    /// Reachable but the account is locked.
    Locked,
}

/// One operation of an unversioned agent surface patch
/// ([`DomainEvent::SurfacePatched`]). Versioned surfaces use
/// [`SurfaceEvent`] instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SurfaceOp {
    /// Append blocks at the end.
    AppendBlocks(Vec<Block>),
    /// Replace the block with the same id.
    ReplaceBlock(Block),
    /// Replace all blocks.
    ReplaceBlocks(Vec<Block>),
    /// Append text to the `"text"` string of a block's data (streaming).
    AppendText {
        /// Target block.
        block_id: BlockId,
        /// Text to append.
        text: String,
    },
    /// Replace all actions.
    SetActions(Vec<SurfaceAction>),
    /// Set the streaming flag.
    SetStreaming(bool),
}

/// Every fact the store understands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DomainEvent {
    /// Account inserted or replaced.
    AccountUpdated(Account),
    /// Node inserted or replaced.
    NodeUpserted(Node),
    /// Health of a known node changed.
    NodeHealthChanged {
        /// Node.
        node_id: NodeId,
        /// New health.
        health: NodeHealth,
    },
    /// Participant inserted or replaced.
    ParticipantUpserted(Participant),
    /// Context inserted or replaced.
    ContextUpserted(Context),
    /// Context and its moments removed.
    ContextRemoved {
        /// Context.
        context_id: ContextId,
    },
    /// New moment.
    MomentAdded(Box<Moment>),
    /// Lifecycle and/or payload of a moment changed. A new payload must be the same kind.
    MomentUpdated {
        /// Moment.
        moment_id: MomentId,
        /// New lifecycle; equal to the current one means unchanged.
        #[serde(default)]
        lifecycle: Option<Lifecycle>,
        /// New payload.
        #[serde(default)]
        kind: Option<MomentKind>,
    },
    /// A versioned agent surface event (create, patch, stream, close).
    Surface(SurfaceEvent),
    /// Agent surface patched in place, without a version (a source that
    /// projects surfaces itself). Raises the surface version by one.
    SurfacePatched {
        /// Moment holding the surface.
        moment_id: MomentId,
        /// Surface; must match the moment's surface.
        surface_id: SurfaceId,
        /// Operations applied in order, all or nothing.
        ops: Vec<SurfaceOp>,
        /// The surface version the patch was made against. When set and
        /// not the current version, the patch is not applied and the surface
        /// is marked in conflict until a full copy replaces it.
        #[serde(default)]
        base_hash: Option<SurfaceHash>,
    },
    /// Object inserted or replaced.
    ObjectUpserted(Object),
    /// New pending grant.
    GrantRequested(Grant),
    /// Pending grant decided.
    GrantResolved {
        /// Grant.
        grant_id: GrantId,
        /// Final state; must not be `Pending`.
        state: GrantState,
    },
    /// The assignee's full task update. The version must rise; state and
    /// step transitions, and where a step needs the requester, are
    /// validated.
    TaskUpserted(Task),
    /// The assignee's update of one step. The version must rise; the task
    /// state follows (a step that needs the requester makes a running task
    /// need them, and the last such step answered makes it run again).
    TaskStepChanged {
        /// Task.
        task_id: TaskId,
        /// The task's next version.
        version: u64,
        /// Step.
        step_id: StepId,
        /// New step state.
        state: StepState,
        /// Where the step asks the requester; required when `state` is
        /// `NeedsMe`, otherwise the step keeps its record.
        #[serde(default)]
        needs_me: Option<NeedsMe>,
    },
    /// The assignee's update of the task state alone (for example the
    /// answer to a cancel). The version must rise. When the task ends, its
    /// unfinished steps are skipped.
    TaskStateChanged {
        /// Task.
        task_id: TaskId,
        /// The task's next version.
        version: u64,
        /// New state.
        state: TaskState,
        /// Result summary, when it changes.
        #[serde(default)]
        result: Option<String>,
    },
    /// A task as its authority holds it, at a version that may skip the
    /// versions in between (a source that reads the whole task, not each
    /// update). An older version is refused; the same version may only
    /// change the flags and the places steps name. Conversation and assignee never change; the
    /// requester is set when a proposal is accepted. Legal transitions
    /// between the shown and the new state are not checked (the steps in
    /// between were not seen); the task's own consistency is.
    TaskSnapshot(Task),
    /// The source's answer to this desktop's request about a task (a
    /// decision on its way, or a refusal). Kept on the task until its
    /// version rises.
    TaskAnswered {
        /// Task.
        task_id: TaskId,
        /// The answer.
        answer: TaskAnswer,
    },
    /// The source no longer holds a task (its history was pruned): the
    /// task and the moments presenting it are dropped.
    TaskRemoved {
        /// Task.
        task_id: TaskId,
    },
    /// Source connectivity changed.
    SourceStatusChanged(SourceStatus),
    /// Local settings of a context changed. `None` fields are unchanged.
    ContextSettingsChanged {
        /// Context.
        context_id: ContextId,
        /// New pin state.
        #[serde(default)]
        pinned: Option<bool>,
        /// New mute state.
        #[serde(default)]
        muted: Option<bool>,
        /// New disappearing timer.
        #[serde(default)]
        disappearing: Option<Disappearing>,
    },
    /// An account was locked or unlocked.
    AccountLockChanged {
        /// Account.
        account_id: AccountId,
        /// New state.
        lock_state: LockState,
    },
    /// Local node settings changed. Setting a default clears the previous one.
    NodeSettingsChanged {
        /// Node.
        node_id: NodeId,
        /// New enabled state.
        #[serde(default)]
        enabled: Option<bool>,
        /// Becomes the default node (only `Some(true)` is meaningful).
        #[serde(default)]
        is_default: Option<bool>,
    },
    /// A node probe finished.
    NodeProbed {
        /// Node.
        node_id: NodeId,
        /// Result.
        probe: NodeProbe,
    },
    /// A node profile was removed.
    NodeRemoved {
        /// Node.
        node_id: NodeId,
    },
    /// Device inserted or replaced (revocation keeps the record).
    DeviceUpserted(Device),
    /// The backend refused a decision on the grant, or the grant stopped
    /// being usable. The grant is closed; it is never retried from here.
    GrantFailed {
        /// Grant.
        grant_id: GrantId,
        /// Why.
        failure: GrantFailure,
    },
    /// An approved grant was revoked before use.
    GrantRevoked {
        /// Grant.
        grant_id: GrantId,
    },
    /// Standing approval inserted or replaced (revocation keeps the record).
    StandingApprovalUpserted(StandingApproval),
    /// One audit record appended.
    AuditAppended(AuditEntry),
    /// The read-only integrations snapshot was replaced.
    IntegrationsUpdated(Integrations),
    /// The backend declared which contract methods it cannot perform.
    CapabilitiesUpdated(Capabilities),
    /// The local daemon reported about itself.
    DaemonInfoUpdated(DaemonInfo),
    /// How the source is kept up to date changed (stream, polling, the
    /// daemon's background sync).
    SyncStatusChanged(SyncStatus),
    /// An effect could not be performed. Changes no domain entity.
    EffectFailed(EffectFailure),
    /// The source answered these collections in full (each may be empty).
    ListingsLoaded(Vec<Listing>),
    /// This account's contact card, as the backend exported it.
    OwnContactCardUpdated(ContactCard),
    /// A voice call as the daemon reports it (the whole call). The version
    /// must rise, the state follow [`crate::CallState::can_transition`], and
    /// the conversation, peer, caller and direction stay. The store keeps
    /// the call's timeline record with it.
    CallUpserted(Call),
    /// The incoming contact requests the daemon lists now (pending ones):
    /// they replace every incoming request the store holds.
    ContactRequestsListed(Vec<ContactRequest>),
    /// One contact request, as it is now (a new one, an answer, an expiry,
    /// or this account's own request to a peer). Keyed by peer and
    /// direction: a peer has at most one request each way.
    ContactRequestUpserted(ContactRequest),
    /// The account's contact request setting.
    ContactRequestPolicyUpdated(RequestPolicy),
    /// The daemon's bound on remembered requester identities, as it is now
    /// (only a daemon that reports it sends this).
    ContactRequestFloorsUpdated(RequestFloors),
    /// The daemon has contact requests but could not list them now (a
    /// locked account, a transient outage, a row it could not read): the
    /// message for the user. The requests already known stay; the next
    /// listing clears it.
    ContactRequestsReadFailed(String),
    /// A full backup or restore job as the daemon reports it (the whole
    /// job). The version must rise, the kind and file stay, and the phase
    /// follow [`crate::BackupPhase::can_transition`]. Boxed: the job's
    /// review and report are large.
    BackupJobUpdated(Box<BackupJob>),
    /// The creation ceremony of a born-v3 account as the daemon reports it
    /// (the whole record, never the recovery phrase). It starts in
    /// `Prepared` (or `Failed` when the daemon refuses), then follows
    /// [`crate::CeremonyPhase::can_follow`]; the ceremony_id and expiry stay.
    CeremonyUpdated(Ceremony),
    /// The recovery ceremony of a born-v3 account as the daemon reports it
    /// (the whole record, never the phrase or the passphrase). It starts in
    /// `Prepared` (or `Failed` when the daemon refuses), then follows
    /// [`crate::RecoveryPhase::can_follow`]; the id and expiry stay.
    RecoveryUpdated(RecoveryCeremony),
    /// The owner of an AI participant: the person's account that custodies
    /// it (the creation ceremony names it; a daemon that lists it reports
    /// it too).
    CustodianKnown {
        /// The AI participant.
        agent: AccountId,
        /// Its custodian.
        custodian: AccountId,
    },
    /// The outcome of exporting an AI participant's credentials: where the
    /// bundle was written, or why it was not. Never the credentials.
    AgentExportUpdated(AgentExport),
}
