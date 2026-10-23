// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! GPUI Kit application layer and views for Ramflux Desktop.
//!
//! Data flows one way (rules R1 and R2):
//!
//! ```text
//! view --UiEvent (bubbling)--> workspace root --Intent--> StoreHandle
//!   --Effect--> source --EventEnvelope--> StoreHandle --Store::apply--> ChangeSet
//!   --StoreEvent--> views that own the changed area
//! ```
//!
//! - [`app`]: [`app::StoreHandle`], the only writer of the store and the
//!   only caller of the source.
//! - [`bubble`]: [`bubble::EventSink`] and the root's intent mapping.
//! - [`confirm`]: the inline hold-to-confirm strip.
//! - [`shell`], [`entry`], [`workspace`], [`settings`], [`dev`]: pages and panels.
//! - [`moments`]: inline moment rows.

#![forbid(unsafe_code)]

rust_i18n::i18n!("locales", fallback = "en");

pub mod actions;
pub mod app;
pub mod bubble;
pub mod confirm;
pub mod dev;
pub mod entry;
pub mod i18n;
mod ids;
pub mod moments;
pub mod route;
pub mod settings;
pub mod shell;
pub mod workspace;

pub use shell::Shell;
pub use workspace::Workspace;

/// Initializes application-level state: locale and key bindings.
///
/// Call after `gpui_kit::init` and before opening windows.
pub fn init(cx: &mut gpui_kit::App) {
    rust_i18n::set_locale(i18n::DEFAULT_LOCALE);
    actions::bind_keys(cx);
}
