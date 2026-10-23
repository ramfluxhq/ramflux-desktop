// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Multi-step tasks started from a one-sentence intent.

use serde::{Deserialize, Serialize};

use super::actor::Actor;
use super::moment::Lifecycle;
use crate::ids::{ContextId, GrantId, TaskId};

/// One step of a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskStep {
    /// Step label.
    pub label: String,
    /// Step lifecycle.
    #[serde(default)]
    pub lifecycle: Lifecycle,
    /// Grant the step waits on when it needs the user.
    #[serde(default)]
    pub grant_id: Option<GrantId>,
}

/// A multi-step unit of work started from an intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// Task id.
    pub id: TaskId,
    /// Opaque correlation id of the originating intent.
    #[serde(default)]
    pub intent_id: Option<String>,
    /// Context the task belongs to.
    pub context_id: ContextId,
    /// Who started it.
    pub actor: Actor,
    /// Title.
    pub title: String,
    /// Ordered steps.
    #[serde(default)]
    pub steps: Vec<TaskStep>,
    /// Task lifecycle.
    #[serde(default)]
    pub lifecycle: Lifecycle,
    /// Result summary once finished.
    #[serde(default)]
    pub result: Option<String>,
}

impl Task {
    /// The first step that needs the user, with its index.
    pub fn needs_me_step(&self) -> Option<(usize, &TaskStep)> {
        self.steps.iter().enumerate().find(|(_, step)| step.lifecycle == Lifecycle::NeedsMe)
    }
}
