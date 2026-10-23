// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Tasks: work a participant (usually an AI participant) does for a
//! requester inside a conversation.
//!
//! A task is started by the requester (`task.start`) or proposed by its
//! assignee and confirmed by the requester. Only the assignee updates it
//! (monotonic `version`); only the requester cancels it. A step that needs
//! the requester points to exactly one place where the decision is made: a
//! grant (approval) or an agent surface (choice or input).

use serde::{Deserialize, Serialize};

use super::actor::Actor;
use super::moment::Lifecycle;
use crate::ids::{ContextId, GrantId, ParticipantId, StepId, SurfaceId, TaskId};

/// State of a task.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum TaskState {
    /// Proposed by the assignee; waits for the requester to confirm.
    #[default]
    Proposed,
    /// The assignee is working on it.
    Running,
    /// A step waits on the requester.
    NeedsMe,
    /// Finished successfully. Terminal.
    Done,
    /// Finished with failure. Terminal.
    Failed,
    /// Cancelled by the requester (or a proposal declined). Terminal.
    Cancelled,
}

impl TaskState {
    /// Returns true for `Done`, `Failed` and `Cancelled`.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }

    /// Returns whether `from -> to` is a legal transition. Self-transitions
    /// are not transitions and return false.
    pub const fn can_transition(from: Self, to: Self) -> bool {
        use TaskState::{Cancelled, Done, Failed, NeedsMe, Proposed, Running};
        matches!(
            (from, to),
            (Proposed, Running | Failed | Cancelled)
                | (Running, NeedsMe | Done | Failed | Cancelled)
                | (NeedsMe, Running | Done | Failed | Cancelled)
        )
    }

    /// The lifecycle of the moments that present a task in this state.
    ///
    /// A proposal needs the requester (confirm or decline); a cancelled
    /// task is dismissed.
    pub const fn moment_lifecycle(self) -> Lifecycle {
        match self {
            Self::Proposed | Self::NeedsMe => Lifecycle::NeedsMe,
            Self::Running => Lifecycle::InProgress,
            Self::Done => Lifecycle::Done,
            Self::Failed => Lifecycle::Failed,
            Self::Cancelled => Lifecycle::Dismissed,
        }
    }
}

/// State of one task step.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum StepState {
    /// Not started.
    #[default]
    Pending,
    /// Running.
    Running,
    /// Waits on the requester.
    NeedsMe,
    /// Finished successfully. Terminal.
    Done,
    /// Finished with failure. Terminal.
    Failed,
    /// Not run (the task ended or no longer needs it). Terminal.
    Skipped,
}

impl StepState {
    /// Returns true for `Done`, `Failed` and `Skipped`.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Skipped)
    }

    /// Returns whether `from -> to` is a legal transition. Self-transitions
    /// are not transitions and return false.
    pub const fn can_transition(from: Self, to: Self) -> bool {
        use StepState::{Done, Failed, NeedsMe, Pending, Running, Skipped};
        matches!(
            (from, to),
            (Pending, Running | NeedsMe | Done | Failed | Skipped)
                | (Running, NeedsMe | Done | Failed | Skipped)
                | (NeedsMe, Running | Done | Failed | Skipped)
        )
    }
}

/// Where a step asks the requester, and how. Exactly one place by
/// construction: an approval is a grant, a choice or an input is an agent
/// surface whose actions carry the answer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum NeedsMe {
    /// Approve or deny a grant (through the grant flow and its hold).
    Approval {
        /// The grant.
        grant_id: GrantId,
    },
    /// Choose among the actions of a surface.
    Choice {
        /// The surface.
        surface_id: SurfaceId,
    },
    /// Fill in and submit a surface form.
    Input {
        /// The surface.
        surface_id: SurfaceId,
    },
}

impl NeedsMe {
    /// The grant of an approval.
    pub fn grant(&self) -> Option<&GrantId> {
        match self {
            Self::Approval { grant_id } => Some(grant_id),
            Self::Choice { .. } | Self::Input { .. } => None,
        }
    }

    /// The surface of a choice or an input.
    pub fn surface(&self) -> Option<&SurfaceId> {
        match self {
            Self::Approval { .. } => None,
            Self::Choice { surface_id } | Self::Input { surface_id } => Some(surface_id),
        }
    }
}

/// One step of a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskStep {
    /// Step id, unique within the task.
    pub id: StepId,
    /// Step label.
    pub label: String,
    /// Step state.
    #[serde(default)]
    pub state: StepState,
    /// Where the step asks (or asked) the requester. Required while the
    /// step is `NeedsMe`; kept afterwards as the record of where the
    /// decision was made.
    #[serde(default)]
    pub needs_me: Option<NeedsMe>,
}

impl TaskStep {
    /// The grant the step is bound to, if it asks for an approval.
    pub fn grant_id(&self) -> Option<&GrantId> {
        self.needs_me.as_ref().and_then(NeedsMe::grant)
    }
}

/// A task in a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// Task id.
    pub id: TaskId,
    /// Opaque correlation id of the originating intent.
    #[serde(default)]
    pub intent_id: Option<String>,
    /// Conversation the task belongs to.
    pub context_id: ContextId,
    /// Who asked for it (the only one who may cancel it).
    pub requester: Actor,
    /// The participant doing the work (the only one who updates it).
    pub assignee: ParticipantId,
    /// Title.
    pub title: String,
    /// Ordered steps.
    #[serde(default)]
    pub steps: Vec<TaskStep>,
    /// Task state.
    #[serde(default)]
    pub state: TaskState,
    /// Monotonic version set by the assignee; every update raises it.
    #[serde(default = "first_version")]
    pub version: u64,
    /// Result summary once finished.
    #[serde(default)]
    pub result: Option<String>,
    /// The surface that holds the full result, if any.
    #[serde(default)]
    pub result_ref: Option<SurfaceId>,
}

const fn first_version() -> u64 {
    1
}

impl Task {
    /// The first step that needs the requester, with its index.
    pub fn needs_me_step(&self) -> Option<(usize, &TaskStep)> {
        self.steps.iter().enumerate().find(|(_, step)| step.state == StepState::NeedsMe)
    }

    /// The step with `id`, with its index.
    pub fn step(&self, id: &StepId) -> Option<(usize, &TaskStep)> {
        self.steps.iter().enumerate().find(|(_, step)| &step.id == id)
    }

    /// Grants the steps are bound to.
    pub fn step_grants(&self) -> impl Iterator<Item = &GrantId> {
        self.steps.iter().filter_map(TaskStep::grant_id)
    }

    /// Number of finished steps (done or skipped).
    pub fn finished_steps(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| matches!(s.state, StepState::Done | StepState::Skipped))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_transition_table() {
        use TaskState::*;
        let all = [Proposed, Running, NeedsMe, Done, Failed, Cancelled];
        let allowed = [
            (Proposed, Running),
            (Proposed, Failed),
            (Proposed, Cancelled),
            (Running, NeedsMe),
            (Running, Done),
            (Running, Failed),
            (Running, Cancelled),
            (NeedsMe, Running),
            (NeedsMe, Done),
            (NeedsMe, Failed),
            (NeedsMe, Cancelled),
        ];
        for from in all {
            for to in all {
                assert_eq!(
                    TaskState::can_transition(from, to),
                    allowed.contains(&(from, to)),
                    "{from:?} -> {to:?}"
                );
            }
        }
        for state in [Done, Failed, Cancelled] {
            assert!(state.is_terminal());
        }
    }

    #[test]
    fn step_transition_table() {
        use StepState::*;
        let all = [Pending, Running, NeedsMe, Done, Failed, Skipped];
        for from in all {
            for to in all {
                let allowed = !from.is_terminal()
                    && from != to
                    && !(to == Pending)
                    && !(from == NeedsMe && to == NeedsMe);
                assert_eq!(StepState::can_transition(from, to), allowed, "{from:?} -> {to:?}");
            }
        }
    }

    #[test]
    fn moments_follow_the_task_state() {
        assert_eq!(TaskState::Proposed.moment_lifecycle(), Lifecycle::NeedsMe);
        assert_eq!(TaskState::Running.moment_lifecycle(), Lifecycle::InProgress);
        assert_eq!(TaskState::NeedsMe.moment_lifecycle(), Lifecycle::NeedsMe);
        assert_eq!(TaskState::Cancelled.moment_lifecycle(), Lifecycle::Dismissed);
    }

    #[test]
    fn needs_me_names_exactly_one_place() {
        let approval = NeedsMe::Approval { grant_id: GrantId::new("g") };
        assert_eq!(approval.grant().map(GrantId::as_str), Some("g"));
        assert!(approval.surface().is_none());
        let choice = NeedsMe::Choice { surface_id: SurfaceId::new("s") };
        assert!(choice.grant().is_none());
        assert_eq!(choice.surface().map(SurfaceId::as_str), Some("s"));
    }
}
