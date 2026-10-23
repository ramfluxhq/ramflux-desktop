// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Tasks: every state transition and reject path, steps that need the
//! requester in exactly one place, moments that follow their task, and the
//! task intents (ask, confirm a proposal, cancel).

mod common;

use common::*;
use ramflux_desktop_core::selectors::{AttentionReason, attention, in_flight};
use ramflux_desktop_core::*;
use serde_json::json;

const NOW: Timestamp = Timestamp(1_000);

fn tid() -> TaskId {
    TaskId::new("t-1")
}

fn seeded() -> Feed {
    let mut feed = Feed::new();
    feed.ok(
        1,
        DomainEvent::ParticipantUpserted(participant(
            "p-ai",
            ParticipantKind::Agent,
            "Orbit",
            false,
        )),
    );
    feed.ok(
        1,
        DomainEvent::ParticipantUpserted(participant(
            "p-lena",
            ParticipantKind::Human,
            "Lena",
            true,
        )),
    );
    let mut ctx = context("c-1", false);
    ctx.participants = vec![ParticipantId::new("p-ai"), ParticipantId::new("p-lena")];
    feed.ok(1, DomainEvent::ContextUpserted(ctx));
    feed.ok(
        1,
        DomainEvent::GrantRequested(Grant {
            id: GrantId::new("g-deploy"),
            scope: "deploy.staging".to_owned(),
            risk: Risk::High,
            single_use: true,
            args_hash: "sha256:00".to_owned(),
            expiry: None,
            state: GrantState::Pending,
            failure: None,
        }),
    );
    feed
}

fn step(id: &str, state: StepState) -> TaskStep {
    TaskStep { id: StepId::new(id), label: id.to_owned(), state, needs_me: None }
}

fn approval(grant: &str) -> NeedsMe {
    NeedsMe::Approval { grant_id: GrantId::new(grant) }
}

fn task(state: TaskState, version: u64, steps: Vec<TaskStep>) -> Task {
    Task {
        id: tid(),
        intent_id: None,
        context_id: ContextId::new("c-1"),
        requester: Actor::Human,
        assignee: ParticipantId::new("p-ai"),
        title: "Ship the launch checklist".to_owned(),
        steps,
        state,
        version,
        result: None,
        result_ref: None,
    }
}

fn proposal() -> Task {
    task(
        TaskState::Proposed,
        1,
        vec![step("notes", StepState::Pending), step("deploy", StepState::Pending)],
    )
}

fn progress_moment() -> DomainEvent {
    DomainEvent::MomentAdded(Box::new(moment(
        "m-task",
        "c-1",
        5,
        MomentKind::TaskProgress(TaskProgressPayload { task_id: tid() }),
        Lifecycle::Pending,
    )))
}

fn step_change(version: u64, id: &str, state: StepState, needs_me: Option<NeedsMe>) -> DomainEvent {
    DomainEvent::TaskStepChanged {
        task_id: tid(),
        version,
        step_id: StepId::new(id),
        state,
        needs_me,
    }
}

fn lifecycle(feed: &Feed) -> Lifecycle {
    feed.store.moment(&MomentId::new("m-task")).expect("task moment").lifecycle
}

fn stored(feed: &Feed) -> Task {
    feed.store.task(&tid()).cloned().expect("task")
}

#[test]
fn a_proposal_runs_needs_me_and_finishes_with_its_moment_following() {
    let mut feed = seeded();
    feed.ok(2, DomainEvent::TaskUpserted(proposal()));
    feed.ok(5, progress_moment());
    assert_eq!(lifecycle(&feed), Lifecycle::NeedsMe, "a proposal waits for the requester");
    let items = attention(&feed.store);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].reason, AttentionReason::TaskProposal);

    let mut running = proposal();
    running.state = TaskState::Running;
    running.version = 2;
    feed.ok(6, DomainEvent::TaskUpserted(running));
    assert_eq!(lifecycle(&feed), Lifecycle::InProgress);
    assert!(attention(&feed.store).is_empty());
    assert_eq!(in_flight(&feed.store), [MomentId::new("m-task")]);

    feed.ok(7, step_change(3, "notes", StepState::Running, None));
    feed.ok(8, step_change(4, "notes", StepState::Done, None));
    let changes =
        feed.ok(9, step_change(5, "deploy", StepState::NeedsMe, Some(approval("g-deploy"))));
    assert!(changes.moments.contains(&MomentId::new("m-task")));
    assert_eq!(
        stored(&feed).state,
        TaskState::NeedsMe,
        "a step that needs me makes the task need me"
    );
    assert_eq!(lifecycle(&feed), Lifecycle::NeedsMe);
    assert_eq!(feed.store.needs_me_since(&MomentId::new("m-task")), Some(Timestamp(9)));
    assert_eq!(attention(&feed.store)[0].reason, AttentionReason::TaskInput);
    let owner = selectors::owning_task_step(&feed.store, &GrantId::new("g-deploy")).expect("owner");
    assert_eq!((owner.moment_id.as_str(), owner.step), ("m-task", 1));

    feed.ok(10, step_change(6, "deploy", StepState::Running, None));
    let t = stored(&feed);
    assert_eq!(t.state, TaskState::Running, "the answered step lets it run again");
    assert_eq!(
        t.steps[1].needs_me,
        Some(approval("g-deploy")),
        "where it was decided stays on record"
    );
    assert_eq!(lifecycle(&feed), Lifecycle::InProgress);
    feed.ok(11, step_change(7, "deploy", StepState::Done, None));
    let mut done = stored(&feed);
    done.state = TaskState::Done;
    done.version = 8;
    done.result = Some("Staging is green".to_owned());
    done.result_ref = Some(SurfaceId::new("s-result"));
    feed.ok(12, DomainEvent::TaskUpserted(done));
    assert_eq!(lifecycle(&feed), Lifecycle::Done);
    assert!(in_flight(&feed.store).is_empty());
    assert_eq!(feed.store.needs_me_since(&MomentId::new("m-task")), None);
}

#[test]
fn versions_must_rise_and_identity_never_changes() {
    let mut feed = seeded();
    feed.ok(2, DomainEvent::TaskUpserted(proposal()));
    assert_eq!(
        feed.push(3, DomainEvent::TaskUpserted(proposal())),
        Err(ApplyError::StaleTaskVersion { task: tid(), current: 1, found: 1 })
    );
    assert_eq!(
        feed.push(4, step_change(1, "notes", StepState::Pending, None)),
        Err(ApplyError::StaleTaskVersion { task: tid(), current: 1, found: 1 })
    );
    let mut moved = proposal();
    moved.version = 2;
    moved.assignee = ParticipantId::new("p-lena");
    assert_eq!(
        feed.push(5, DomainEvent::TaskUpserted(moved)),
        Err(ApplyError::TaskIdentityChanged(tid()))
    );
    let mut by_agent = proposal();
    by_agent.version = 2;
    by_agent.requester = Actor::Agent(ParticipantId::new("p-ai"));
    assert_eq!(
        feed.push(6, DomainEvent::TaskUpserted(by_agent)),
        Err(ApplyError::TaskIdentityChanged(tid()))
    );
    let mut zero = proposal();
    zero.id = TaskId::new("t-0");
    zero.version = 0;
    assert!(matches!(
        feed.push(7, DomainEvent::TaskUpserted(zero)),
        Err(ApplyError::InvalidTask { .. })
    ));
    let mut nowhere = proposal();
    nowhere.id = TaskId::new("t-2");
    nowhere.context_id = ContextId::new("c-none");
    assert_eq!(
        feed.push(8, DomainEvent::TaskUpserted(nowhere)),
        Err(ApplyError::UnknownContext(ContextId::new("c-none")))
    );
    assert_eq!(stored(&feed), proposal(), "nothing changed");
}

#[test]
fn illegal_task_and_step_transitions_are_refused() {
    let mut feed = seeded();
    feed.ok(
        2,
        DomainEvent::TaskUpserted(task(TaskState::Running, 1, vec![step("a", StepState::Done)])),
    );
    let back = task(TaskState::Proposed, 2, vec![step("a", StepState::Pending)]);
    assert_eq!(
        feed.push(3, DomainEvent::TaskUpserted(back)),
        Err(ApplyError::IllegalTaskTransition {
            task: tid(),
            from: TaskState::Running,
            to: TaskState::Proposed
        })
    );
    let reopened = task(TaskState::Running, 2, vec![step("a", StepState::Running)]);
    assert_eq!(
        feed.push(4, DomainEvent::TaskUpserted(reopened)),
        Err(ApplyError::IllegalStepTransition {
            task: tid(),
            step: StepId::new("a"),
            from: StepState::Done,
            to: StepState::Running,
        })
    );
    assert_eq!(
        feed.push(5, step_change(2, "zz", StepState::Done, None)),
        Err(ApplyError::UnknownStep { task: tid(), step: StepId::new("zz") })
    );
    let other = DomainEvent::TaskStepChanged {
        task_id: TaskId::new("t-none"),
        version: 2,
        step_id: StepId::new("a"),
        state: StepState::Done,
        needs_me: None,
    };
    assert_eq!(feed.push(6, other), Err(ApplyError::UnknownTask(TaskId::new("t-none"))));
    feed.ok(
        7,
        DomainEvent::TaskUpserted(task(TaskState::Cancelled, 2, vec![step("a", StepState::Done)])),
    );
    let revived = task(TaskState::Running, 3, vec![step("a", StepState::Done)]);
    assert!(matches!(
        feed.push(8, DomainEvent::TaskUpserted(revived)),
        Err(ApplyError::IllegalTaskTransition { from: TaskState::Cancelled, .. })
    ));
}

#[test]
fn needs_me_is_in_exactly_one_place() {
    let mut feed = seeded();
    let invalid = |feed: &mut Feed, t: Task, problem: &str| match feed
        .push(2, DomainEvent::TaskUpserted(t))
    {
        Err(ApplyError::InvalidTask { problem: found, .. }) => assert_eq!(found, problem),
        other => panic!("expected {problem}, got {other:?}"),
    };
    invalid(
        &mut feed,
        task(TaskState::NeedsMe, 1, vec![step("a", StepState::NeedsMe)]),
        "a step that needs the requester must say where",
    );
    invalid(
        &mut feed,
        task(TaskState::NeedsMe, 1, vec![step("a", StepState::Running)]),
        "a task that needs the requester must have a step that does",
    );
    let mut asking = step("a", StepState::NeedsMe);
    asking.needs_me = Some(approval("g-deploy"));
    invalid(
        &mut feed,
        task(TaskState::Running, 1, vec![asking.clone()]),
        "only a task that needs the requester may have a step that does",
    );
    let mut twin = asking.clone();
    twin.id = StepId::new("b");
    invalid(
        &mut feed,
        task(TaskState::NeedsMe, 1, vec![asking.clone(), twin]),
        "two steps ask in the same place",
    );
    invalid(
        &mut feed,
        task(TaskState::Proposed, 1, vec![step("a", StepState::Running)]),
        "a proposal has not started",
    );
    invalid(
        &mut feed,
        task(
            TaskState::Running,
            1,
            vec![step("a", StepState::Pending), step("a", StepState::Pending)],
        ),
        "two steps share an id",
    );
    assert!(feed.store.task(&tid()).is_none());
    // A step change without a place is refused the same way.
    feed.ok(
        3,
        DomainEvent::TaskUpserted(task(TaskState::Running, 1, vec![step("a", StepState::Running)])),
    );
    assert!(matches!(
        feed.push(4, step_change(2, "a", StepState::NeedsMe, None)),
        Err(ApplyError::InvalidTask { .. })
    ));
}

#[test]
fn a_choice_needs_me_at_its_surface_not_at_the_task() {
    let mut feed = seeded();
    feed.ok(
        2,
        DomainEvent::TaskUpserted(task(
            TaskState::Running,
            1,
            vec![step("pick", StepState::Running)],
        )),
    );
    feed.ok(5, progress_moment());
    feed.ok(
        6,
        DomainEvent::Surface(SurfaceEvent::Create {
            moment_id: MomentId::new("m-pick"),
            context_id: ContextId::new("c-1"),
            origin: ParticipantId::new("p-ai"),
            surface_id: SurfaceId::new("s-pick"),
            version: 1,
            blocks: vec![Block::new("b-q", BlockKind::Text, json!({ "text": "Which day?" }))],
            actions: Vec::new(),
            fallback_text: "Orbit asks which day to release".to_owned(),
        }),
    );
    let choice = NeedsMe::Choice { surface_id: SurfaceId::new("s-pick") };
    feed.ok(7, step_change(2, "pick", StepState::NeedsMe, Some(choice)));
    let items = attention(&feed.store);
    assert_eq!(items.len(), 1, "one decision, one place");
    assert_eq!(items[0].moment_id.as_str(), "m-pick");
    assert_eq!(items[0].reason, AttentionReason::SurfaceInput);
    let text = export::timeline_text(&feed.store, &ContextId::new("c-1")).join("\n");
    assert!(
        text.contains("needs you: choose in \u{201c}Orbit asks which day to release\u{201d}"),
        "{text}"
    );
}

#[test]
fn a_task_moment_ignores_lifecycle_updates_while_its_task_is_known() {
    let mut feed = seeded();
    feed.ok(2, DomainEvent::TaskUpserted(task(TaskState::Running, 1, Vec::new())));
    feed.ok(5, progress_moment());
    feed.ok(6, set_lifecycle("m-task", Lifecycle::Done));
    assert_eq!(lifecycle(&feed), Lifecycle::InProgress);
}

fn intent(feed: &Feed, intent: Intent) -> Result<Vec<Effect>, IntentError> {
    handle_intent(&feed.store, intent, NOW)
}

fn call(effects: Vec<Effect>) -> (String, JsonValue) {
    let effect = effects.into_iter().next().expect("one effect");
    (effect.method().expect("call").to_owned(), effect.body().cloned().expect("body"))
}

#[test]
fn asking_an_ai_participant_starts_a_task_in_the_protocol_shape() {
    let feed = seeded();
    let ask = |assignee: &str, text: &str| {
        Intent::human(IntentKind::StartTask {
            context_id: ContextId::new("c-1"),
            assignee: ParticipantId::new(assignee),
            text: text.to_owned(),
        })
    };
    let (method, body) = call(intent(&feed, ask("p-ai", "Sort the checklist")).expect("ok"));
    assert_eq!(method, methods::TASK_START);
    assert_eq!(body["conversation_id"], "c-1");
    assert_eq!(body["assignee"], "p-ai");
    assert_eq!(body["text"], "Sort the checklist");
    assert_eq!(body["proposal_id"], JsonValue::Null);
    assert_eq!(body["proposed_by"], "Human");
    assert!(matches!(intent(&feed, ask("p-lena", "x")), Err(IntentError::NotAnAgent(_))));
    assert!(matches!(intent(&feed, ask("p-ai", "  ")), Err(IntentError::Empty(_))));
    assert!(matches!(intent(&feed, ask("p-none", "x")), Err(IntentError::UnknownParticipant(_))));
    let mut outside = seeded();
    outside.ok(
        2,
        DomainEvent::ParticipantUpserted(participant(
            "p-bot",
            ParticipantKind::Agent,
            "Bot",
            false,
        )),
    );
    assert!(matches!(intent(&outside, ask("p-bot", "x")), Err(IntentError::NotAMember { .. })));
}

#[test]
fn only_the_human_requester_confirms_a_proposal_and_cancels() {
    let mut feed = seeded();
    feed.ok(2, DomainEvent::TaskUpserted(proposal()));
    let accept = || IntentKind::AcceptTaskProposal { task_id: tid() };
    let (method, body) = call(intent(&feed, Intent::human(accept())).expect("ok"));
    assert_eq!(method, methods::TASK_START);
    assert_eq!(
        (body["proposal_id"].as_str(), body["assignee"].as_str()),
        (Some("t-1"), Some("p-ai"))
    );
    assert_eq!(body["text"], "Ship the launch checklist");
    assert!(
        matches!(
            intent(&feed, Intent::agent(ParticipantId::new("p-ai"), accept())),
            Err(IntentError::AgentForbidden(_))
        ),
        "agents never confirm"
    );
    let cancel = || IntentKind::CancelTask { task_id: tid() };
    let (method, body) = call(intent(&feed, Intent::human(cancel())).expect("decline"));
    assert_eq!((method.as_str(), body["version"].as_u64()), (methods::TASK_CANCEL, Some(1)));
    assert!(
        matches!(
            intent(&feed, Intent::agent(ParticipantId::new("p-ai"), cancel())),
            Err(IntentError::NotRequester(_))
        ),
        "the assignee cannot cancel what the requester asked"
    );

    let mut running = proposal();
    running.state = TaskState::Running;
    running.version = 2;
    feed.ok(3, DomainEvent::TaskUpserted(running.clone()));
    assert!(matches!(intent(&feed, Intent::human(accept())), Err(IntentError::NotAProposal(_))));
    running.state = TaskState::Done;
    running.version = 3;
    feed.ok(4, DomainEvent::TaskUpserted(running));
    assert!(matches!(intent(&feed, Intent::human(cancel())), Err(IntentError::TaskFinished(_))));
}

#[test]
fn a_state_update_ends_the_task_and_skips_its_unfinished_steps() {
    let mut feed = seeded();
    let mut asking = step("deploy", StepState::NeedsMe);
    asking.needs_me = Some(approval("g-deploy"));
    feed.ok(
        2,
        DomainEvent::TaskUpserted(task(
            TaskState::NeedsMe,
            3,
            vec![step("notes", StepState::Done), asking],
        )),
    );
    feed.ok(5, progress_moment());
    assert_eq!(lifecycle(&feed), Lifecycle::NeedsMe);
    let cancelled = |version| DomainEvent::TaskStateChanged {
        task_id: tid(),
        version,
        state: TaskState::Cancelled,
        result: Some("Cancelled by Alex".to_owned()),
    };
    assert!(matches!(feed.push(6, cancelled(3)), Err(ApplyError::StaleTaskVersion { .. })));
    feed.ok(7, cancelled(100));
    let t = stored(&feed);
    assert_eq!(t.state, TaskState::Cancelled);
    assert_eq!(t.steps[0].state, StepState::Done);
    assert_eq!(t.steps[1].state, StepState::Skipped);
    assert_eq!(t.result.as_deref(), Some("Cancelled by Alex"));
    assert_eq!(lifecycle(&feed), Lifecycle::Dismissed);
    assert!(attention(&feed.store).is_empty());
    let back = DomainEvent::TaskStateChanged {
        task_id: tid(),
        version: 101,
        state: TaskState::Running,
        result: None,
    };
    assert!(matches!(feed.push(8, back), Err(ApplyError::IllegalTaskTransition { .. })));
    let unknown = DomainEvent::TaskStateChanged {
        task_id: TaskId::new("t-none"),
        version: 2,
        state: TaskState::Done,
        result: None,
    };
    assert_eq!(feed.push(9, unknown), Err(ApplyError::UnknownTask(TaskId::new("t-none"))));
}
