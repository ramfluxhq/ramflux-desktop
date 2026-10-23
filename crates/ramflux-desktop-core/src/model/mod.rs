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
//! - [`object`]: transferable objects.
//! - [`grant`]: grants, standing approvals and the audit trail.
//! - [`task`]: multi-step tasks.
//! - [`integrations`]: read-only MCP and A2I snapshots.

mod account;
mod actor;
mod backend;
mod conversation;
mod grant;
mod integrations;
mod moment;
mod object;
mod surface;
mod task;

pub use account::{
    Account, AccountKind, Device, LockState, Node, NodeHealth, NodeProbe, PinSource, ProbeOutcome,
};
pub use actor::{Actor, ConfirmGesture, ConfirmationIssuer, HumanConfirmation, MIN_HOLD_MS};
pub use backend::{Capabilities, DaemonInfo, EffectFailure, FailureCode};
pub use conversation::{
    Context, ContextKind, Disappearing, Participant, ParticipantKind, Role, SafetyState,
};
pub use grant::{AuditEntry, Grant, GrantState, Risk, StandingApproval};
pub use integrations::{A2iItem, Integrations, McpServer};
pub use moment::{
    AgentSurfacePayload, ConflictPayload, ConflictResolution, ConflictSide, ContactRequestPayload,
    FallbackPayload, GovernanceAction, GovernancePayload, HandoffPayload, InterventionPayload,
    Lifecycle, MessagePayload, Moment, MomentKind, ObjectTransferPayload, PrivacyRevealPayload,
    Receipt, RevealRecord, SafetyChangeKind, SafetyChangePayload, SystemLevel, SystemPayload,
    TaskProgressPayload,
};
pub use object::{Direction, Object, TransferState};
pub use surface::{Block, BlockKind, SurfaceAction};
pub use task::{Task, TaskStep};
