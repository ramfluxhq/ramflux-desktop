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
//! - [`card_event`] holds the card runtime's facts; [`store::CardPool`]
//!   keeps the card sessions within the runtime's limits.
//! - [`recovery`] holds the recovery phrase of a born-v3 account on its way to
//!   the screen, and the word list typed words are checked against.
//! - [`export`] renders a conversation as plain text (the terminal degradation).
//!
//! See `docs/contract.md` for the full description.

#![forbid(unsafe_code)]
// Production code: no unwrap, expect, panic or `std::sync` locks (see the
// workspace `Cargo.toml`); tests may.
#![cfg_attr(
    not(test),
    deny(clippy::disallowed_types, clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

pub mod card_event;
pub mod event;
pub mod export;
pub mod ids;
pub mod intent;
pub mod model;
pub mod policy;
pub mod projection;
pub mod recovery;
pub mod selectors;
pub mod source;
pub mod store;
pub mod surface_event;
pub mod surface_hash;
pub mod typing;
pub mod ui_event;

pub use card_event::{
    CARD_CHANGED_REASON, CARD_METHODS, CARD_RUNTIME_GAP, CardEvent, CardLink, CardReceipt,
    CardSlotOutcome, CardStatus, https_host,
};
pub use event::{
    DomainEvent, EVENT_SCHEMA, EventEnvelope, SourceStatus, SurfaceEvent, SurfaceSnapshot,
};
pub use ids::{
    AccountId, BackupJobId, BlockId, CallId, ContextId, DeviceId, GrantId, MomentId, NodeId,
    ObjectId, ParticipantId, StandingApprovalId, StepId, SurfaceId, TaskId, Timestamp,
};
pub use intent::{
    AppLockPhase, CardInput, ContactAction, ContactCardProblem, ContactIdProblem, ContactInput,
    Effect, GovernanceOp, Intent, IntentKind, MAX_CONTACT_CARD_LEN, MAX_CONTACT_ID_LEN,
    SecretInput, SyncDirection, check_contact_card, check_contact_id, classify_contact_input,
    methods,
};
pub use model::*;
pub use policy::{
    GroupProblem, IntentError, MAX_GROUP_MEMBERS, MAX_GROUP_TITLE_LEN, call_peer,
    check_add_contact, check_add_contact_card, check_agent_custodian, check_create_group,
    check_group_title, check_start_call, check_start_conversation, contact_action_current,
    handle_intent,
};
pub use recovery::{
    PHRASE_WORDS, PhraseCheck, PhraseError, RecoveryPhrase, WordInput, check_phrase,
    normalize_phrase, resolve_word, suggestions,
};
pub use selectors::{AttentionItem, AttentionReason, CONTEXT_STACK_CAP, GrantFilter, TaskStepRef};
pub use serde_json::Value as JsonValue;
pub use source::Source;
pub use store::{
    ApplyError, BodyCache, CardDue, CardRecord, Changes, MAX_AUDIT, MAX_BODIES, MAX_FAILURES,
    MAX_REJECTIONS, Rejection, Store, SurfaceRecord, SurfaceState, UpdateRequest,
};
pub use surface_hash::SurfaceHash;
pub use typing::{TYPING_MAX_MS, TypingStore};
pub use ui_event::{NodeAction, ObjectAction, Scope, UiEvent, UiEventKind};
