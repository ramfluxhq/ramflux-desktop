// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The Ramflux Desktop data contract.
//!
//! Pure Rust, no GPUI and no async runtime (rule R3). Data flows one way
//! (rule R2):
//!
//! ```text
//! UiEvent -> Intent -> Effect -> Source -> EventEnvelope -> Store -> selectors -> view
//! ```
//!
//! - [`Store`] is the single writer and only changes through [`Store::apply`] (R1).
//! - [`selectors`] derive read-only views.
//! - [`handle_intent`] validates intents and applies policy without mutating the store.
//! - [`Source`] executes effects and yields events.
//!
//! See `docs/contract.md` for the full description.

#![forbid(unsafe_code)]

pub mod event;
pub mod ids;
pub mod intent;
pub mod model;
pub mod policy;
pub mod selectors;
pub mod source;
pub mod store;
pub mod ui_event;

pub use event::{DomainEvent, EVENT_SCHEMA, EventEnvelope, SourceStatus, SurfaceOp};
pub use ids::{
    AccountId, BlockId, ContextId, DeviceId, GrantId, MomentId, NodeId, ObjectId, ParticipantId,
    StandingApprovalId, SurfaceId, TaskId, Timestamp,
};
pub use intent::{Effect, GovernanceOp, Intent, IntentKind, SecretInput, SyncDirection, methods};
pub use model::*;
pub use policy::{IntentError, handle_intent};
pub use selectors::{AttentionItem, AttentionReason, CONTEXT_STACK_CAP, GrantFilter, TaskStepRef};
pub use serde_json::Value as JsonValue;
pub use source::Source;
pub use store::{
    ApplyError, BodyCache, Changes, MAX_AUDIT, MAX_BODIES, MAX_FAILURES, MAX_REJECTIONS, Rejection,
    Store,
};
pub use ui_event::{NodeAction, ObjectAction, Scope, UiEvent, UiEventKind};
