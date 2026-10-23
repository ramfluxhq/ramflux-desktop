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
//! - [`config`]: socket, deadlines, poll intervals.
//! - [`bus`]: requests, failures, deadlines, retry keys, the multiplexed
//!   request connection, the subscription.
//! - [`bridge`]: the bridge thread and one task per scope.
//! - [`plan`]: effects to daemon requests, and the reads.
//! - [`translate`]: daemon answers and events to domain events.
//! - [`capabilities`]: what the daemon cannot do yet, with reasons.
//! - [`diagnostics`]: the redacted diagnostics bundle.
//! - [`pins`]: pinned contexts persisted in the desktop settings file.
//! - [`local`]: files, launch settings, a started daemon and the app log.
//! - [`source`]: the `Source` implementation.

#![forbid(unsafe_code)]

pub mod bridge;
pub mod bus;
pub mod capabilities;
pub mod config;
pub mod diagnostics;
pub mod local;
pub mod pins;
pub mod plan;
pub mod source;
pub mod translate;

pub use config::{DEFAULT_SOCKET, Deadlines, SdkConfig};

/// The product revision the `ramflux-sdk`, `ramflux-protocol` and
/// `ramflux-crypto` dependencies are pinned to (checked against
/// `Cargo.lock` by a test).
pub const PRODUCT_REV: &str = "b38b4e414b24f1172b6672e6845ba960f409d6df";
pub use source::{DaemonStop, SdkHub, SdkSource};
pub use translate::Scope;
