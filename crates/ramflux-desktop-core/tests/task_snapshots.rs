// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Task snapshots from a source that reads whole tasks from their
//! authority: skipped versions, flags at the same version, a proposal's
//! requester set at acceptance, finished tasks frozen, answers kept until
//! the version rises; and the task bodies the policy sends (the version
//! seen, the conversation, decline), requester-only decisions on a task's
//! approval, and members who never propose.

mod common;

use common::*;
use ramflux_desktop_core::*;

fn tid() -> TaskId {
    TaskId::new("t-1")
}

fn seeded() -> Feed {
    let mut feed = Feed::new();
    let agent = participant("p-ai", ParticipantKind::Agent, "Orbit", false);
    feed.ok(1, DomainEvent::ParticipantUpserted(agent));
    let mut ctx = context("c-1", false);
    ctx.participants = vec![ParticipantId::new("p-ai")];
    feed.ok(1, DomainEvent::ContextUpserted(ctx));
    feed.ok(1, DomainEvent::GrantRequested(grant("g-1", Risk::Low)));
    feed
}

fn step(id: &str, state: StepState) -> TaskStep {
    TaskStep { id: StepId::new(id), label: id.to_owned(), state, needs_me: None }
}

fn task(state: TaskState, version: u64, steps: Vec<TaskStep>) -> Task {
    Task {
        id: tid(),
        intent_id: None,
        context_id: ContextId::new("c-1"),
        requester: Actor::Human,
        assignee: ParticipantId::new("p-ai"),
        title: "Check".to_owned(),
        steps,
        state,
        version,
        result: None,
        result_ref: None,
        flags: TaskFlags::default(),
        answer: None,
    }
}

fn stored(feed: &Feed) -> Task {
    feed.store.task(&tid()).cloned().expect("task")
}

fn snapshot(task: Task) -> DomainEvent {
    DomainEvent::TaskSnapshot(task)
}

#[test]
fn snapshots_skip_versions_and_refuse_older_ones() {
    let mut feed = seeded();
    feed.ok(2, snapshot(task(TaskState::Running, 2, vec![step("a", StepState::Pending)])));
    // From pending straight to done: the steps in between were not seen.
    feed.ok(3, snapshot(task(TaskState::Done, 7, vec![step("a", StepState::Done)])));
    assert_eq!((stored(&feed).version, stored(&feed).state), (7, TaskState::Done));
    let older =
        feed.push(4, snapshot(task(TaskState::Running, 5, vec![step("a", StepState::Running)])));
    assert!(matches!(older, Err(ApplyError::StaleTaskVersion { current: 7, found: 5, .. })));
    // A finished task does not advance.
    let after = feed.push(5, snapshot(task(TaskState::Running, 8, Vec::new())));
    assert!(matches!(after, Err(ApplyError::IllegalTaskTransition { .. })));
    // An inconsistent task is refused as from any other event.
    let mut waiting = task(TaskState::NeedsMe, 9, vec![step("a", StepState::Running)]);
    waiting.id = TaskId::new("t-2");
    assert!(matches!(feed.push(6, snapshot(waiting)), Err(ApplyError::InvalidTask { .. })));
}

#[test]
fn the_same_version_changes_only_flags_and_places() {
    let mut feed = seeded();
    let mut waiting = step("a", StepState::NeedsMe);
    waiting.needs_me = Some(NeedsMe::Approval { grant_id: GrantId::new("grant_raw") });
    feed.ok(2, snapshot(task(TaskState::NeedsMe, 3, vec![waiting.clone()])));
    // The place resolves to the approval the desktop knows, and the
    // assignee is stalled: same version.
    let mut resolved = task(TaskState::NeedsMe, 3, vec![waiting]);
    resolved.steps[0].needs_me = Some(NeedsMe::Approval { grant_id: GrantId::new("g-1") });
    resolved.flags.stalled = true;
    feed.ok(3, snapshot(resolved));
    let now = stored(&feed);
    assert!(now.flags.stalled);
    assert_eq!(now.steps[0].grant_id(), Some(&GrantId::new("g-1")));
    // A different state needs a new version.
    let moved =
        feed.push(4, snapshot(task(TaskState::Running, 3, vec![step("a", StepState::Running)])));
    assert!(matches!(moved, Err(ApplyError::StaleTaskVersion { .. })));
}

#[test]
fn a_proposal_gets_its_requester_when_accepted_and_no_other_change_moves_it() {
    let mut feed = seeded();
    feed.ok(2, snapshot(task(TaskState::Proposed, 1, vec![step("a", StepState::Pending)])));
    let mut accepted = task(TaskState::Running, 2, vec![step("a", StepState::Pending)]);
    accepted.requester = Actor::Member(ParticipantId::new("p-ana"));
    feed.ok(3, snapshot(accepted));
    assert_eq!(stored(&feed).requester, Actor::Member(ParticipantId::new("p-ana")));
    let mut moved = task(TaskState::Running, 3, vec![step("a", StepState::Running)]);
    moved.requester = Actor::Human;
    assert!(matches!(feed.push(4, snapshot(moved)), Err(ApplyError::TaskIdentityChanged(_))));
    let mut reassigned = task(TaskState::Running, 3, vec![step("a", StepState::Running)]);
    reassigned.requester = Actor::Member(ParticipantId::new("p-ana"));
    reassigned.assignee = ParticipantId::new("p-other");
    assert!(matches!(feed.push(5, snapshot(reassigned)), Err(ApplyError::TaskIdentityChanged(_))));
}

#[test]
fn an_answer_stays_until_the_version_rises() {
    let mut feed = seeded();
    let unknown = feed.push(
        2,
        DomainEvent::TaskAnswered {
            task_id: tid(),
            answer: TaskAnswer::Refused(TaskRefusal::Changed),
        },
    );
    assert!(matches!(unknown, Err(ApplyError::UnknownTask(_))));
    feed.ok(3, snapshot(task(TaskState::Running, 2, Vec::new())));
    let pending = TaskAnswer::DecisionPending { step_id: StepId::new("a") };
    let changes = feed.ok(4, DomainEvent::TaskAnswered { task_id: tid(), answer: pending.clone() });
    assert!(changes.tasks.contains(&tid()));
    let mut flagged = task(TaskState::Running, 2, Vec::new());
    flagged.flags.compacted = true;
    feed.ok(5, snapshot(flagged));
    assert_eq!(stored(&feed).answer, Some(pending), "kept at the same version");
    feed.ok(6, snapshot(task(TaskState::Running, 3, Vec::new())));
    assert_eq!(stored(&feed).answer, None, "the next version answers it");
}

#[test]
fn bodies_carry_the_version_seen_and_the_conversation() {
    let mut feed = seeded();
    feed.ok(2, snapshot(task(TaskState::Proposed, 4, Vec::new())));
    let call = |store: &Store, kind| {
        let effects = handle_intent(store, Intent::human(kind), Timestamp(0)).expect("ok");
        let effect = effects.into_iter().next().expect("one");
        (effect.method().expect("call").to_owned(), effect.body().cloned().expect("body"))
    };
    let (method, body) = call(&feed.store, IntentKind::AcceptTaskProposal { task_id: tid() });
    assert_eq!((method.as_str(), body["version"].as_u64()), (methods::TASK_START, Some(4)));
    let (method, body) = call(&feed.store, IntentKind::CancelTask { task_id: tid() });
    assert_eq!(method, methods::TASK_CANCEL);
    assert_eq!(
        (body["decline"].as_bool(), body["conversation_id"].as_str()),
        (Some(true), Some("c-1"))
    );
    feed.ok(3, snapshot(task(TaskState::Running, 5, Vec::new())));
    let (_, body) = call(&feed.store, IntentKind::CancelTask { task_id: tid() });
    assert_eq!((body["decline"].as_bool(), body["version"].as_u64()), (Some(false), Some(5)));
}

#[test]
fn a_step_of_someone_elses_task_is_not_decided_here_and_members_never_propose() {
    let mut feed = seeded();
    let mut waiting = step("a", StepState::NeedsMe);
    waiting.needs_me = Some(NeedsMe::Approval { grant_id: GrantId::new("g-1") });
    let mut theirs = task(TaskState::NeedsMe, 2, vec![waiting]);
    theirs.requester = Actor::Member(ParticipantId::new("p-ana"));
    feed.ok(2, snapshot(theirs));
    let approve = Intent::human(IntentKind::Approve { grant_id: GrantId::new("g-1") });
    let refused = handle_intent(&feed.store, approve, Timestamp(0));
    assert!(matches!(refused, Err(IntentError::NotRequester(_))));
    let cancel = Intent::human(IntentKind::CancelTask { task_id: tid() });
    assert!(matches!(
        handle_intent(&feed.store, cancel, Timestamp(0)),
        Err(IntentError::NotRequester(_))
    ));
    let member = Intent {
        proposed_by: Actor::Member(ParticipantId::new("p-ana")),
        confirmed_by: None,
        kind: IntentKind::CancelTask { task_id: tid() },
    };
    assert!(matches!(
        handle_intent(&feed.store, member, Timestamp(0)),
        Err(IntentError::UnknownAgent(_))
    ));
}

#[test]
fn the_text_view_says_what_the_flags_say() {
    let mut feed = seeded();
    let mut stalled = task(TaskState::Running, 2, Vec::new());
    stalled.flags.stalled = true;
    stalled.flags.compacted = true;
    feed.ok(2, snapshot(stalled));
    let text = export::task_text(&feed.store, &stored(&feed));
    assert!(text.contains("stalled by Orbit"), "{text}");
    assert!(text.contains("history compacted"), "{text}");
    assert!(text.contains("(actor kind unverified)"), "{text}");
}
