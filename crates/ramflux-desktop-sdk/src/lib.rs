// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The SDK-backed desktop source: the desktop on a real `rfd`.
//!
//! [`SdkHub`] starts one bridge thread (the only tokio runtime in the
//! application) and hands out an [`SdkSource`] per scope: the daemon (entry
//! page) and each unlocked account. Sources implement the core `Source`
//! trait and talk to the bridge over runtime-agnostic channels; the GPUI side
//! never awaits a tokio future.
//!
//! - [`config`]: socket, deadlines, the polling fallback and its intervals.
//! - [`bus`]: requests, failures, deadlines, retry keys, the multiplexed
//!   request connection, the event stream and the push subscription.
//! - [`bridge`]: the bridge thread and one task per scope.
//! - [`plan`]: effects to daemon requests, and the reads.
//! - [`translate`]: daemon answers and events to domain events.
//! - [`capabilities`]: what the daemon cannot do yet, with reasons.
//! - [`diagnostics`]: the redacted diagnostics bundle.
//! - [`directory`]: what the client directories list that requests need.
//! - [`pins`]: pinned contexts persisted in the desktop settings file.
//! - [`cursor`]: event-stream cursors persisted in the desktop settings file.
//! - [`local`]: files, launch settings, a started daemon and the app log.
//! - [`source`]: the `Source` implementation.

#![forbid(unsafe_code)]

pub mod bridge;
pub mod bus;
pub mod capabilities;
pub mod config;
pub mod cursor;
pub mod diagnostics;
pub mod directory;
pub mod local;
pub mod pins;
pub mod plan;
pub mod source;
pub mod translate;

pub use config::{DEFAULT_SOCKET, Deadlines, SdkConfig};
pub use plan::conversation::fresh_conversation_id;
pub use plan::message::ATTACHMENT_LIMIT;

/// The product revision the `ramflux-sdk`, `ramflux-protocol` and
/// `ramflux-crypto` dependencies are pinned to (checked against
/// `Cargo.lock` by a test).
pub const PRODUCT_REV: &str = "9227749eb27725d40aab9e7e92abf173fcb3a4f5";
pub use source::{DaemonStop, SdkHub, SdkSource};
pub use translate::Scope;
