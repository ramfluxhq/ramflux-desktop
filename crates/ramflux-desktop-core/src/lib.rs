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
//! - [`conformance`] checks agent surfaces against a renderer profile.
//! - [`export`] renders a conversation as plain text (the terminal degradation).
//!
//! See `docs/contract.md` for the full description.

#![forbid(unsafe_code)]

pub mod conformance;
pub mod event;
pub mod export;
pub mod ids;
pub mod intent;
pub mod model;
pub mod policy;
pub mod selectors;
pub mod source;
pub mod store;
pub mod surface_event;
pub mod ui_event;

pub use conformance::{
    RendererError, RendererErrorKind, RendererProfile, SurfaceHash, SurfaceVerdict, surface_hash,
};
pub use event::{
    DomainEvent, EVENT_SCHEMA, EventEnvelope, SourceStatus, SurfaceEvent, SurfaceOp, SurfacePatchOp,
};
pub use ids::{
    AccountId, BlockId, ContextId, DeviceId, GrantId, MomentId, NodeId, ObjectId, ParticipantId,
    StandingApprovalId, StepId, SurfaceId, TaskId, Timestamp,
};
pub use intent::{
    ContactAction, ContactIdProblem, Effect, GovernanceOp, Intent, IntentKind, MAX_CONTACT_ID_LEN,
    SecretInput, SyncDirection, check_contact_id, methods,
};
pub use model::*;
pub use policy::{
    IntentError, check_add_contact, check_start_conversation, contact_action_current, handle_intent,
};
pub use selectors::{AttentionItem, AttentionReason, CONTEXT_STACK_CAP, GrantFilter, TaskStepRef};
pub use serde_json::Value as JsonValue;
pub use source::Source;
pub use store::{
    ApplyError, BodyCache, Changes, MAX_AUDIT, MAX_BODIES, MAX_FAILURES, MAX_REJECTIONS, Rejection,
    Store, StreamState, SurfaceRecord, SurfaceState,
};
pub use ui_event::{NodeAction, ObjectAction, Scope, UiEvent, UiEventKind};
