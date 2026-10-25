// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Normalized domain model.
//!
//! The store keeps every entity keyed by its id. Moments reference their
//! context; surfaces, grants, objects and tasks are referenced by id from the
//! moment payloads that present them.
//!
//! - [`actor`]: who proposes and who confirms.
//! - [`backend`]: backend capabilities, daemon info and failed effects.
//! - [`account`]: accounts, nodes and devices.
//! - [`conversation`]: participants and contexts.
//! - [`moment`]: moments, their lifecycle and every payload kind.
//! - [`surface`]: agent surface blocks and actions.
//! - [`protected`]: revealed protected text that never prints.
//! - [`object`]: transferable objects.
//! - [`grant`]: grants, standing approvals and the audit trail.
//! - [`task`]: tasks, their steps and where a step needs the requester.
//! - [`integrations`]: read-only MCP and A2I snapshots.
//! - [`listing`]: whether a collection was listed yet (loading vs empty).

mod account;
mod actor;
mod backend;
mod conversation;
mod grant;
mod integrations;
mod listing;
mod moment;
mod object;
mod protected;
mod surface;
mod task;

pub use account::{
    Account, AccountKind, Device, LockState, Node, NodeHealth, NodeProbe, PinSource, ProbeOutcome,
};
pub use actor::{Actor, ConfirmGesture, ConfirmationIssuer, HumanConfirmation, MIN_HOLD_MS};
pub use backend::{
    Capabilities, DaemonInfo, DaemonSyncState, EffectFailure, FailureCode, SyncMode, SyncStatus,
};
pub use conversation::{
    ContactCard, ContactLink, Context, ContextKind, Disappearing, Participant, ParticipantKind,
    Role, RootSource, SafetyState,
};
pub use grant::{AuditEntry, Grant, GrantFailure, GrantState, Risk, StandingApproval};
pub use integrations::{A2iItem, Integrations, McpServer};
pub use listing::{ListState, Listing};
pub use moment::{
    AgentSurfacePayload, ConflictPayload, ConflictResolution, ConflictSide, ContactRequestPayload,
    FallbackPayload, GovernanceAction, GovernancePayload, HandoffPayload, InterventionPayload,
    Lifecycle, MessagePayload, Moment, MomentKind, ObjectTransferPayload, PrivacyRevealPayload,
    Receipt, RevealRecord, SafetyChangeKind, SafetyChangePayload, SystemLevel, SystemPayload,
    TaskEventKind, TaskEventPayload, TaskProgressPayload,
};
pub use object::{Direction, Object, TransferState};
pub use protected::Protected;
pub use surface::{Block, BlockKind, DEFAULT_BLOCK_VERSION, SurfaceAction};
pub use task::{NeedsMe, StepState, Task, TaskAnswer, TaskFlags, TaskRefusal, TaskState, TaskStep};
