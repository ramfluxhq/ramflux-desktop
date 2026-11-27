// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Agent surface actions (declarative, never code).
//!
//! A surface's content is a card, drawn by the card runtime (see
//! `crate::card_event` and `store::cards`). The only thing a surface still
//! offers the desktop as data is its actions: a task step's decision is
//! answered through them (`surface.action`).

use serde::{Deserialize, Serialize};

use super::grant::Risk;

/// An action offered by a surface. Invoking it becomes an intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceAction {
    /// Action id, unique within its surface.
    pub id: String,
    /// Button label.
    pub label: String,
    /// Declared risk.
    #[serde(default)]
    pub risk: Risk,
    /// When set, the action is shown disabled with this reason.
    #[serde(default)]
    pub disabled_reason: Option<String>,
}
