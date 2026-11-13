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

/// Active generated palette, selected by the installed native theme.
#[must_use]
pub fn palette(cx: &gpui_kit::App) -> generated::rf_tokens::RFTheme {
    use gpui_kit::component::ActiveTheme as _;
    if cx.theme().is_dark() {
        generated::rf_tokens::RFTheme::dark()
    } else {
        generated::rf_tokens::RFTheme::light()
    }
}
