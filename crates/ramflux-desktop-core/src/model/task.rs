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
use super::conversation::PeerKind;
use super::moment::Lifecycle;
use crate::ids::{ContextId, DeviceId, GrantId, ParticipantId, StepId, SurfaceId, TaskId};

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
    /// What the source reports about the task beyond its state.
    #[serde(default)]
    pub flags: TaskFlags,
    /// Who acts on the task, with how far each one's kind is proven.
    #[serde(default)]
    pub actors: TaskActors,
    /// The source's answer to this desktop's last request about the task
    /// (a decision on its way, or a refusal), until the task changes.
    #[serde(default)]
    pub answer: Option<TaskAnswer>,
}

/// What the source reports about a task beyond its state and steps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskFlags {
    /// The assignee used up what it may send: its further updates are
    /// refused and the task stays where it is. The requester can still
    /// cancel it.
    #[serde(default)]
    pub stalled: bool,
    /// The task's history was compacted: its outcome and steps are kept,
    /// the events that led there are not.
    #[serde(default)]
    pub compacted: bool,
}

/// A role on a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskRole {
    /// Who asked for the task.
    Requester,
    /// Who does the work.
    Assignee,
    /// Who accepted it (the assignee a started task, the requester a
    /// proposal).
    Acceptor,
}

/// One actor of a task, as the source authenticated it: its principal,
/// the device its last event came from, and its kind with whether a
/// root-signed profile proves it for the role (a signed bot or headless-AI
/// profile never authenticates a human role). Metadata only, never a
/// permission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskActor {
    /// The principal.
    pub participant_id: ParticipantId,
    /// The device its last event came from; `None` when no event device is
    /// known.
    #[serde(default)]
    pub device_id: Option<DeviceId>,
    /// The kind its public profile states, when the source holds one.
    #[serde(default)]
    pub kind: Option<PeerKind>,
    /// Whether a root-signed profile and a current device of the principal
    /// prove the kind for this role.
    #[serde(default)]
    pub kind_verified: bool,
}

/// The actors of a task by role; a role nobody holds yet is `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskActors {
    /// Who asked.
    #[serde(default)]
    pub requester: Option<TaskActor>,
    /// Who does the work.
    #[serde(default)]
    pub assignee: Option<TaskActor>,
    /// Who accepted.
    #[serde(default)]
    pub acceptor: Option<TaskActor>,
}

impl TaskActors {
    /// The actor in `role`.
    pub const fn get(&self, role: TaskRole) -> Option<&TaskActor> {
        match role {
            TaskRole::Requester => self.requester.as_ref(),
            TaskRole::Assignee => self.assignee.as_ref(),
            TaskRole::Acceptor => self.acceptor.as_ref(),
        }
    }
}

/// Why the source refused a request about a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskRefusal {
    /// The task changed since it was shown (an older revision, or the
    /// decision it answered is no longer open): review it again.
    Changed,
    /// The task has finished.
    Finished,
    /// This account may not do that (only the requester decides, cancels
    /// or resolves; agents never confirm).
    NotAllowed,
    /// That decision was already used.
    AlreadyDecided,
    /// The request is larger than a task message may be.
    TooLarge,
    /// The assignee could not confirm the decision.
    DecisionRejected,
    /// The task is stalled (it reached what it may hold); the requester
    /// can still cancel or resolve it.
    Stalled,
    /// Too many requests for now (a rate limit); trying later may work.
    Busy,
    /// A task quota is used up: trying again only helps once a task
    /// finishes or is cancelled (never retried on its own).
    QuotaExhausted,
    /// The node could not be reached for now; trying again may work.
    Unreachable,
    /// The node refused the session as unauthenticated: not retried
    /// until the account signs in again.
    Unauthenticated,
    /// The node's TLS certificate or pin did not verify: nothing is
    /// retried until the trusted binding is repaired.
    Untrusted,
    /// Another reason, as the source gave it.
    Other(String),
}

/// The source's answer to this desktop's request about a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskAnswer {
    /// The decision on a step was sent; the assignee confirms it once it
    /// has checked the grant or the surface answer.
    DecisionPending {
        /// The step decided.
        step_id: StepId,
    },
    /// The decision was sent but the assignee has not confirmed it in
    /// time; it stays pending (the assignee may still commit it) and the
    /// task can still be cancelled.
    DecisionUnconfirmed {
        /// The step decided.
        step_id: StepId,
    },
    /// The decision on a step was signed but could not be delivered to
    /// the assignee (its retries ran out): sending the same decision again
    /// retries it unchanged.
    DecisionUndelivered {
        /// The step decided.
        step_id: StepId,
        /// The approval decided.
        grant_id: GrantId,
        /// Whether it was approved (else denied).
        approved: bool,
    },
    /// The request was refused; nothing changed.
    Refused(TaskRefusal),
}

const fn first_version() -> u64 {
    1
}

impl Task {
    /// Whether the assignee's kind is proven by a root-signed profile
    /// (false until the source reports the actors).
    pub fn assignee_kind_verified(&self) -> bool {
        self.actors.assignee.as_ref().is_some_and(|actor| actor.kind_verified)
    }

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
    fn the_assignee_kind_is_verified_only_when_its_actor_says_so() {
        let mut actors = TaskActors::default();
        assert!(actors.get(TaskRole::Assignee).is_none());
        let actor = TaskActor {
            participant_id: ParticipantId::new("p-orbit"),
            device_id: None,
            kind: Some(PeerKind::HeadlessAi),
            kind_verified: true,
        };
        actors.assignee = Some(actor.clone());
        assert_eq!(actors.get(TaskRole::Assignee), Some(&actor));
        assert!(actors.get(TaskRole::Requester).is_none());
        let task = Task {
            id: TaskId::new("t"),
            intent_id: None,
            context_id: ContextId::new("c"),
            requester: Actor::Human,
            assignee: ParticipantId::new("p-orbit"),
            title: String::new(),
            steps: Vec::new(),
            state: TaskState::Running,
            version: 1,
            result: None,
            result_ref: None,
            flags: TaskFlags::default(),
            actors,
            answer: None,
        };
        assert!(task.assignee_kind_verified());
        let unreported = Task { actors: TaskActors::default(), ..task };
        assert!(!unreported.assignee_kind_verified(), "unverified until reported");
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
