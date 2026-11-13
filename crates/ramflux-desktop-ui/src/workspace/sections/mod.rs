// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Workspace sections beside the conversation list: groups, contacts and
//! strangers. Each section owns a searchable directory (the list pane) and,
//! where the design has one, a detail view. Sections read the rendered
//! account's store, emit UI events through the root sink and report
//! navigation to the workspace; they never change the store.
//!
//! - [`directory`]: the shared search field, rows and states.
//! - [`groups`]: the group directory (C05, states C32–C35).
//! - [`contacts`]: the contact directory and profile with the safety number
//!   (F family, states F28–F30).
//! - [`strangers`]: incoming requests from non-contacts (F17, F21): reply
//!   is not acceptance, batch decisions, block.

pub(crate) mod contact_profile;
pub mod contacts;
pub(crate) mod directory;
pub mod groups;
pub(crate) mod stranger_detail;
pub mod strangers;

pub use contacts::{ContactsDirectory, ContactsDirectoryEvent};
pub use groups::{GroupDirectory, GroupDirectoryEvent};
pub use strangers::{StrangersArea, StrangersEvent};
