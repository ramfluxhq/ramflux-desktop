// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Domain events: facts flowing from a source into the store.

use serde::{Deserialize, Serialize};

use crate::ids::{
    AccountId, BlockId, ContextId, GrantId, MomentId, NodeId, SurfaceId, TaskId, Timestamp,
};
use crate::model::{
    Account, AuditEntry, Block, Capabilities, Context, DaemonInfo, Device, Disappearing,
    EffectFailure, Grant, GrantState, Integrations, Lifecycle, LockState, Moment, MomentKind, Node,
    NodeHealth, NodeProbe, Object, Participant, StandingApproval, SurfaceAction, Task,
};

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

/// One patch operation on an agent surface.
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
    /// Agent surface patched in place.
    SurfacePatched {
        /// Moment holding the surface.
        moment_id: MomentId,
        /// Surface; must match the moment's surface.
        surface_id: SurfaceId,
        /// Operations applied in order, all or nothing.
        ops: Vec<SurfaceOp>,
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
    /// Task inserted or replaced; lifecycle changes are validated.
    TaskUpserted(Task),
    /// One task step changed lifecycle.
    TaskStepChanged {
        /// Task.
        task_id: TaskId,
        /// Zero-based step index.
        index: u32,
        /// New step lifecycle.
        lifecycle: Lifecycle,
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
    /// An effect could not be performed. Changes no domain entity.
    EffectFailed(EffectFailure),
}
