// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Controlled RF navigation presentation; hosts own routes and business actions.

pub mod geometry;
pub mod row;
pub mod theme;

/// Imported presentation catalog; client code owns state and authority.
// The compiler preserves source decimal precision and uniform geometry scaling.
// Keep these two generated-data lints local to the manifest-owned import.
#[allow(clippy::excessive_precision, clippy::eq_op)]
#[rustfmt::skip]
pub mod generated;

/// Active generated palette: the active theme's token set (see
/// [`theme::palette`]).
#[must_use]
pub fn palette(cx: &gpui_kit::App) -> generated::rf_tokens::RFTheme {
    theme::palette(cx)
}
