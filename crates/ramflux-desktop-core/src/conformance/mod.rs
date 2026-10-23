// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Renderer conformance: what a renderer declares it can show, and how a
//! surface is checked against that declaration before anything renders.
//!
//! - [`profile`]: the renderer profile (device class, block kinds and
//!   versions, input and output modes, surface modes, limits).
//! - [`hash`]: the canonical content hash every surface version carries.
//! - [`schema`]: the data shape each block kind needs.
//! - [`check`]: a surface against a profile: limits, unsupported blocks,
//!   shape mismatches, display-blocked blocks.
//! - [`error`]: the renderer error record and the effect that reports it.
//!
//! A surface over a limit renders as a whole fallback (never partially) and
//! offers no active actions; an unsupported, malformed or display-blocked
//! block renders as a block fallback. Each case is a [`RendererError`].

pub mod check;
pub mod error;
pub mod hash;
pub mod profile;
pub mod schema;

pub use check::{BlockIssue, SurfaceVerdict, check_surface, over_limit};
pub use error::{RendererError, RendererErrorKind};
pub use hash::{SurfaceHash, surface_hash};
pub use profile::{
    BlockSupport, DeviceClass, InputMode, LimitKind, Limits, OutputMode, RendererProfile,
    SurfaceMode,
};
