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
//! - [`contact_request`]: contact requests and the held messages they
//!   carry, and the account's request setting.
//! - [`backup`]: full backup and restore jobs.
//! - [`ceremony`]: the creation ceremony of a born-v3 account (never the
//!   recovery phrase).
//! - [`call`]: voice calls, their states and outcomes, and the secret
//!   media session that never enters the store.

mod account;
mod actor;
mod backend;
mod backup;
mod call;
mod ceremony;
mod contact_request;
mod conversation;
mod grant;
mod integrations;
mod listing;
mod moment;
mod object;
mod protected;
mod recovery_ceremony;
mod surface;
mod task;

pub use account::{
    Account, AccountKind, Device, LockState, Node, NodeHealth, NodeProbe, PinSource, ProbeOutcome,
    SecureSessions,
};
pub use actor::{Actor, ConfirmGesture, ConfirmationIssuer, HumanConfirmation, MIN_HOLD_MS};
pub use backend::{
    Capabilities, DaemonInfo, DaemonSyncState, EffectFailure, FailureCode, SyncMode, SyncStatus,
};
pub use backup::{
    AttachmentCheck, BackupCategory, BackupCounts, BackupFailure, BackupJob, BackupJobKind,
    BackupPhase, BackupProgress, BackupSummary, RestorePreview, RestoreReport,
};
pub use call::{
    Call, CallDirection, CallEndReason, CallKey, CallMedia, CallOutcome, CallRebind, CallState,
};
pub use ceremony::{
    CEREMONY_TTL_MS, CHALLENGE_WORDS, Ceremony, CeremonyAccount, CeremonyFailure, CeremonyPhase,
    Networking,
};
pub use contact_request::{
    ContactRequest, RequestAnswer, RequestDirection, RequestFloors, RequestPolicy, RequestState,
};
pub use conversation::{
    ContactCard, ContactLink, Context, ContextKind, Disappearing, KindClaim, NameWarnings,
    Participant, ParticipantKind, PeerKind, Role, RootSource, SafetyState,
};
pub use grant::{AuditEntry, Grant, GrantFailure, GrantState, Risk, StandingApproval};
pub use integrations::{A2iItem, Integrations, McpServer};
pub use listing::{ListState, Listing};
pub use moment::{
    AgentSurfacePayload, CallPayload, ConflictPayload, ConflictResolution, ConflictSide,
    ContactRequestPayload, FallbackPayload, GovernanceAction, GovernancePayload,
    GroupRenamePayload, HandoffPayload, InterventionPayload, Lifecycle, MessagePayload, Moment,
    MomentKind, NameChangePayload, ObjectTransferPayload, PrivacyRevealPayload, Receipt,
    RevealRecord, SafetyChangeKind, SafetyChangePayload, SystemLevel, SystemPayload, TaskEventKind,
    TaskEventPayload, TaskProgressPayload,
};
pub use object::{Direction, Object, TransferState};
pub use protected::Protected;
pub use recovery_ceremony::{RecoveryCeremony, RecoveryPhase};
pub use surface::{Block, BlockKind, DEFAULT_BLOCK_VERSION, SurfaceAction};
pub use task::{
    NeedsMe, StepState, Task, TaskActor, TaskActors, TaskAnswer, TaskFlags, TaskRefusal, TaskRole,
    TaskState, TaskStep,
};
