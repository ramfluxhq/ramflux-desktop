// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Conversation tasks on the SDK source, against a fake daemon that holds
//! them on the protocol's own state machine with signed events (see
//! `ramflux_desktop_test::fake_tasks`): `task.changed` calls for `task.get`
//! and replays change nothing, tasks are scoped by conversation, starting,
//! accepting, declining and cancelling use the daemon's bodies and the
//! version the user saw, an approval resolves its step through the grant
//! flow (pending until the assignee confirms; a replay is refused), stale
//! views come back as "changed", a stalled assignee and a compacted history
//! are shown, task messages are not shown as messages, and s10 (demo 3)
//! replays through the daemon to the same result as on the mock source.

mod common;

use common::task_world::*;
use ramflux_desktop_core::Source as _;
use ramflux_desktop_core::{
    Actor, ConfirmationIssuer, ContextId, Effect, GrantId, IntentKind, ParticipantId, StepId,
    StepState, SyncMode, TaskAnswer, TaskRefusal, TaskState, Timestamp, handle_intent, methods,
};
use ramflux_desktop_test::fake_tasks::{approval_step, task_step};
use ramflux_protocol::TaskStatus;
use serde_json::json;

#[test]
fn a_proposal_is_shown_once_and_accepted_with_the_version_seen() {
    let (world, path) = World::start("accept");
    // Proposed before the account opens, so it is listed and not read on
    // its own: no read of it can still be on its way when the replay below
    // is counted (proposals arriving live are shown by the next test).
    world.tasks.propose(GROUP, "t-1", "Check the release", &[("a", "Read the notes")]).unwrap();
    let mut account = open(&path);
    account.until("the proposal on the open stream", |s, _| {
        task(s, GROUP, "t-1").is_some()
            && s.sync_status().is_some_and(|sync| sync.mode == SyncMode::Stream)
    });
    assert!(world.fake.streaming(ACCOUNT));
    let shown = task(&account.store, GROUP, "t-1").expect("task");
    assert_eq!((shown.state, shown.version), (TaskState::Proposed, 1));
    assert_eq!(shown.requester, Actor::Human, "this account may accept it");
    assert_eq!(shown.assignee, ParticipantId::new(BOT));
    assert_eq!(shown.steps[0].state, StepState::Pending);
    assert!(!shown.flags.kind_verified, "kinds are asserted until they are signed");
    let moment = ramflux_desktop_sdk::translate::tasks::task_moment_id(&shown.id);
    assert!(account.store.moment(&moment).is_some(), "the task has its moment");

    // The same metadata again changes nothing and reads nothing.
    let mark = world.mark();
    let meta = json!({ "task_id": "t-1", "conversation_id": GROUP, "version": 1,
                       "state": "proposed", "actor_kind_verified": false });
    world.fake.publish(ACCOUNT, "task.changed", meta);
    account.settle(300);
    assert!(world.since(mark, "task.get").is_empty(), "a replayed change reads nothing");

    act(&mut account, human(IntentKind::AcceptTaskProposal { task_id: shown.id.clone() }));
    account.until("running", |s, _| {
        task(s, GROUP, "t-1").is_some_and(|t| t.state == TaskState::Running)
    });
    let accept = world.since(0, "task.accept").pop().expect("task.accept");
    assert_eq!(accept.body["conversation_id"], GROUP);
    assert_eq!(accept.body["task_id"], "t-1");
    assert_eq!(accept.body["version"], 2);
    assert!(accept.body["message_id"].is_string());
    assert!(account.store.rejections().is_empty(), "{:?}", account.store.rejections());
    world.fake.shutdown();
}

#[test]
fn a_declined_proposal_is_cancelled_and_another_members_acceptance_is_changed() {
    let (world, path) = World::start("decline");
    let mut account = open(&path);
    world.tasks.propose(GROUP, "t-1", "One", &[]).unwrap();
    world.tasks.propose(GROUP, "t-2", "Two", &[]).unwrap();
    account
        .until("both", |s, _| task(s, GROUP, "t-1").is_some() && task(s, GROUP, "t-2").is_some());
    act(&mut account, human(IntentKind::CancelTask { task_id: key(GROUP, "t-1") }));
    account.until("declined", |s, _| {
        task(s, GROUP, "t-1").is_some_and(|t| t.state == TaskState::Cancelled)
    });
    let decline = world.since(0, "task.decline").pop().expect("task.decline");
    assert_eq!(
        (decline.body["task_id"].as_str(), decline.body["version"].as_u64()),
        (Some("t-1"), Some(2))
    );

    // Ana accepts t-2 a moment before this account does: the desktop still
    // shows the proposal, its acceptance is refused as changed, and the
    // task read back is Ana's.
    let id = key(GROUP, "t-2");
    world.tasks.set_quiet(true);
    world.tasks.accept_as("principal-ana", GROUP, "t-2").unwrap();
    world.tasks.set_quiet(false);
    assert_eq!(account.store.task(&id).map(|t| t.state), Some(TaskState::Proposed));
    act(&mut account, human(IntentKind::AcceptTaskProposal { task_id: id.clone() }));
    account.until("Ana's task, to review", |s, _| {
        s.task(&id).is_some_and(|t| {
            t.state == TaskState::Running
                && t.requester == Actor::Member(ParticipantId::new("principal-ana"))
                && t.answer == Some(TaskAnswer::Refused(TaskRefusal::Changed))
        })
    });
    let accept = world.since(0, "task.accept").pop().expect("task.accept");
    assert_eq!(accept.body["version"], 2, "the version shown plus one");
    assert!(account.store.failures().is_empty(), "answered on the task");
    // Not this account's: it cannot cancel it.
    let refused =
        handle_intent(&account.store, human(IntentKind::CancelTask { task_id: id }), Timestamp(0));
    assert!(refused.is_err(), "only the requester cancels");
    world.fake.shutdown();
}

#[test]
fn a_task_started_from_the_composer_runs_and_is_cancelled_by_its_requester() {
    let (world, path) = World::start("start");
    let mut account = open(&path);
    let intent = human(IntentKind::StartTask {
        context_id: ContextId::new(GROUP),
        assignee: ParticipantId::new(BOT),
        text: "Summarize the open launch items".into(),
    });
    act(&mut account, intent);
    account.until("started", |s, _| s.tasks().any(|t| t.state == TaskState::Running));
    let start = world.since(0, "task.start").pop().expect("task.start");
    assert_eq!(start.body["conversation_id"], GROUP);
    assert_eq!(start.body["assignee"], BOT);
    assert_eq!(start.body["text"], "Summarize the open launch items");
    let task_id = start.body["task_id"].as_str().expect("task id").to_owned();
    assert!(start.body["message_id"].is_string());
    let id = key(GROUP, &task_id);
    assert_eq!(account.store.task(&id).map(|t| t.requester.clone()), Some(Actor::Human));

    world
        .tasks
        .update(
            GROUP,
            &task_id,
            TaskStatus::Running,
            vec![task_step("a", "Read", TaskStatus::Running)],
            None,
        )
        .unwrap();
    account.until("a step", |s, _| s.task(&id).is_some_and(|t| t.steps.len() == 1));
    act(&mut account, human(IntentKind::CancelTask { task_id: id.clone() }));
    account.until("cancelled", |s, _| s.task(&id).is_some_and(|t| t.state == TaskState::Cancelled));
    let cancel = world.since(0, "task.cancel").pop().expect("task.cancel");
    assert_eq!(cancel.body["version"], 2, "the version the user saw");
    assert_eq!(cancel.body["conversation_id"], GROUP);
    let cancelled = account.store.task(&id).cloned().expect("task");
    assert_eq!(cancelled.steps[0].state, StepState::Skipped);
    // Cancelling again: the task has finished.
    account.source.submit(Effect::Call {
        method: methods::TASK_CANCEL.into(),
        body: json!({ "task_id": id, "version": 2, "decline": false }),
    });
    account.until("finished", |s, _| {
        s.task(&id).is_some_and(|t| t.answer == Some(TaskAnswer::Refused(TaskRefusal::Finished)))
    });
    assert!(account.store.failures().is_empty(), "answered on the task, not the status strip");
    world.fake.shutdown();
}

#[test]
fn an_approval_resolves_its_step_through_the_grant_flow_once() {
    let (world, path) = World::start("approve");
    let mut account = open(&path);
    world.tasks.start_as("principal-me", GROUP, "t-1", BOT, "Deploy to staging").unwrap();
    world.approvals.lock().unwrap().push(approval("appr-7", "high"));
    world.fake.publish(ACCOUNT, "mcp.approval.request", json!({ "approval_id": "appr-7" }));
    account.until("the approval", |s, _| s.grant(&GrantId::new("appr-7")).is_some());
    let steps = vec![approval_step("deploy", "Deploy to staging", "grant_appr-7")];
    world.tasks.update(GROUP, "t-1", TaskStatus::NeedsMe, steps, None).unwrap();
    let id = key(GROUP, "t-1");
    account.until("needs me", |s, _| s.task(&id).is_some_and(|t| t.state == TaskState::NeedsMe));
    let shown = account.store.task(&id).cloned().expect("task");
    assert_eq!(shown.steps[0].grant_id(), Some(&GrantId::new("appr-7")), "decided by its approval");

    // The hold confirms the grant; the step's resolution follows it.
    let mut intent = human(IntentKind::Approve { grant_id: GrantId::new("appr-7") });
    intent.confirmed_by = ConfirmationIssuer::new().issue_hold(1_200);
    act(&mut account, intent);
    account.until("pending", |s, _| {
        s.task(&id).is_some_and(|t| {
            t.answer == Some(TaskAnswer::DecisionPending { step_id: StepId::new("deploy") })
        })
    });
    let resolve = world.since(0, "task.resolve").pop().expect("task.resolve");
    assert_eq!(resolve.body["base_revision"], shown.version);
    assert_eq!(resolve.body["step_id"], "deploy");
    assert_eq!(resolve.body["target"], json!({ "kind": "approval", "grant_id": "grant_appr-7" }));
    assert_eq!(resolve.body["decision_id"], decision_id("appr-7"), "the grant's decision id");
    assert_eq!(resolve.body["decision"], "resume");
    assert_eq!(world.tasks.pending(), 1);

    // The assignee checks the grant and confirms: the step runs again.
    assert_eq!(world.tasks.confirm_decisions(), 1);
    account.until("running", |s, _| {
        s.task(&id).is_some_and(|t| t.state == TaskState::Running && t.answer.is_none())
    });
    assert_eq!(account.store.task(&id).map(|t| t.steps[0].state), Some(StepState::Running));

    // The same decision again (by the scoped approval id, which the daemon
    // canonicalizes to the decision id) is refused on the task.
    account.source.submit(Effect::Call {
        method: methods::TASK_RESOLVE.into(),
        body: json!({
            "task_id": id, "base_revision": shown.version, "step_id": "deploy",
            "target": { "kind": "approval", "grant_id": "grant_appr-7" },
            "decision_id": "appr-7", "decision": "resume",
        }),
    });
    account.until("replay refused", |s, _| {
        s.task(&id)
            .is_some_and(|t| t.answer == Some(TaskAnswer::Refused(TaskRefusal::AlreadyDecided)))
    });
    assert!(account.store.rejections().is_empty(), "{:?}", account.store.rejections());
    world.fake.shutdown();
}

#[test]
fn a_denied_approval_fails_the_task_and_someone_elses_step_is_not_decided_here() {
    let (world, path) = World::start("deny");
    let mut account = open(&path);
    world.tasks.start_as("principal-me", GROUP, "mine", BOT, "Deploy").unwrap();
    world.tasks.start_as("principal-ana", GROUP, "anas", BOT, "Rotate keys").unwrap();
    for id in ["appr-1", "appr-2"] {
        world.approvals.lock().unwrap().push(approval(id, "low"));
    }
    world.fake.publish(ACCOUNT, "mcp.approval.request", json!({ "approval_id": "appr-1" }));
    account.until("approvals", |s, _| s.grant(&GrantId::new("appr-2")).is_some());
    let step = |grant: &str| vec![approval_step("go", "Go", grant)];
    world.tasks.update(GROUP, "mine", TaskStatus::NeedsMe, step("grant_appr-1"), None).unwrap();
    world.tasks.update(GROUP, "anas", TaskStatus::NeedsMe, step("grant_appr-2"), None).unwrap();
    let (mine, anas) = (key(GROUP, "mine"), key(GROUP, "anas"));
    account.until("both need someone", |s, _| {
        [&mine, &anas].iter().all(|id| s.task(id).is_some_and(|t| t.state == TaskState::NeedsMe))
    });
    let refused = handle_intent(
        &account.store,
        human(IntentKind::Approve { grant_id: GrantId::new("appr-2") }),
        Timestamp(0),
    );
    assert!(refused.is_err(), "Ana's step is decided by Ana");

    act(&mut account, human(IntentKind::Deny { grant_id: GrantId::new("appr-1") }));
    account.until("pending", |s, _| s.task(&mine).is_some_and(|t| t.answer.is_some()));
    let resolve = world.since(0, "task.resolve").pop().expect("task.resolve");
    assert_eq!(resolve.body["decision"], "deny");
    world.tasks.confirm_decisions();
    account.until("failed", |s, _| s.task(&mine).is_some_and(|t| t.state == TaskState::Failed));
    assert_eq!(world.since(0, "task.resolve").len(), 1, "nothing resolved for Ana's step");
    world.fake.shutdown();
}

#[test]
fn a_stalled_assignee_and_a_compacted_history_are_shown() {
    let (world, path) = World::start("stalled");
    let mut account = open(&path);
    world.tasks.start_as("principal-me", GROUP, "t-1", BOT, "Index the archive").unwrap();
    world.tasks.start_as("principal-me", GROUP, "t-2", BOT, "Tidy up").unwrap();
    let (one, two) = (key(GROUP, "t-1"), key(GROUP, "t-2"));
    account.until("both", |s, _| s.task(&one).is_some() && s.task(&two).is_some());
    world.tasks.stall(GROUP, "t-1");
    account.until("stalled", |s, _| s.task(&one).is_some_and(|t| t.flags.stalled));
    assert!(
        world.tasks.update(GROUP, "t-1", TaskStatus::Running, Vec::new(), None).is_err(),
        "its updates are refused"
    );
    // The requester can still cancel it.
    act(&mut account, human(IntentKind::CancelTask { task_id: one.clone() }));
    account
        .until("cancelled", |s, _| s.task(&one).is_some_and(|t| t.state == TaskState::Cancelled));

    let done = vec![task_step("a", "Tidy", TaskStatus::Done)];
    world.tasks.update(GROUP, "t-2", TaskStatus::Done, done, Some("s-result")).unwrap();
    account.until("done", |s, _| s.task(&two).is_some_and(|t| t.state == TaskState::Done));
    // Compaction is not announced: the conversation's next read shows it.
    world.tasks.compact(GROUP, "t-2");
    account.source.focus(None);
    account.source.focus(Some(&ContextId::new(GROUP)));
    account.until("compacted", |s, _| s.task(&two).is_some_and(|t| t.flags.compacted));
    let compacted = account.store.task(&two).cloned().expect("t-2");
    assert_eq!(compacted.steps.len(), 1, "the outcome and steps are kept");
    assert_eq!(compacted.result_ref.as_ref().map(|s| s.as_str()), Some("s-result"));
    assert!(account.store.rejections().is_empty(), "{:?}", account.store.rejections());
    world.fake.shutdown();
}

#[test]
fn tasks_are_scoped_by_conversation_and_their_messages_are_not_messages() {
    let (world, path) = World::start("scoped");
    let mut account = open(&path);
    world.tasks.start_as("principal-me", GROUP, "same", BOT, "In the group").unwrap();
    world.tasks.start_as("principal-me", "dm-ana", "same", BOT, "In the direct one").unwrap();
    account.source.focus(Some(&ContextId::new("dm-ana")));
    account.until("both", |s, _| {
        task(s, GROUP, "same").is_some() && task(s, "dm-ana", "same").is_some()
    });
    assert_eq!(task(&account.store, GROUP, "same").map(|t| t.title), Some("In the group".into()));
    assert_eq!(
        task(&account.store, "dm-ana", "same").map(|t| t.title),
        Some("In the direct one".into())
    );
    assert!(!world.tasks.messages("dm-ana").is_empty());
    account.settle(200);
    let shown: Vec<String> = account
        .store
        .context_moments(&ContextId::new("dm-ana"))
        .filter_map(|m| match &m.kind {
            ramflux_desktop_core::MomentKind::Message(_) => Some(m.id.to_string()),
            _ => None,
        })
        .collect();
    assert!(shown.iter().all(|id| !id.contains("task-msg")), "task messages: {shown:?}");
    world.fake.shutdown();
}
