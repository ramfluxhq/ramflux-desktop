// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! s10 (demo 3) on the SDK path: the scenario's task, played through the
//! fake daemon on the protocol's state machine. The AI participant's
//! proposal and updates are its signed events; the user's two decisions
//! (start the proposal, approve the deploy with a hold) go through the core
//! policy and the SDK as on the real daemon. Every version the scenario
//! shows is shown on the SDK path with the same state and steps; the
//! deploy step is decided through its approval, and the task ends done
//! with its result surface (s10 carries no result summary).

mod common;

use common::task_world::*;
use ramflux_desktop_core::{
    ConfirmationIssuer, DomainEvent, GrantId, IntentKind, NeedsMe, StepState, Store, Task, TaskId,
    TaskState,
};
use ramflux_desktop_test::fake_tasks::approval_step;
use ramflux_protocol::{TaskStatus, TaskStep};
use serde_json::json;

const TASK: &str = "t-launch";
const GRANT: &str = "g-deploy";

/// The scenario's task as the mock source shows it after each event.
fn mirrored() -> Vec<Task> {
    let scenario = ramflux_desktop_test::load_fixture("s10_ai_participant").expect("s10");
    let events = scenario.responses.iter().take(3).flat_map(|r| r.emit.iter().map(|(_, e)| e));
    let mut task: Option<Task> = None;
    let mut shown = Vec::new();
    for event in events {
        match (event, task.as_mut()) {
            (DomainEvent::TaskUpserted(t), _) => task = Some(t.clone()),
            (DomainEvent::TaskStateChanged { version, state, .. }, Some(t)) => {
                t.version = *version;
                t.state = *state;
            }
            (DomainEvent::TaskStepChanged { version, step_id, state, needs_me, .. }, Some(t)) => {
                t.version = *version;
                if let Some(step) = t.steps.iter_mut().find(|s| &s.id == step_id) {
                    step.state = *state;
                    if needs_me.is_some() {
                        step.needs_me.clone_from(needs_me);
                    }
                }
                let waiting = t.steps.iter().any(|s| s.state == StepState::NeedsMe);
                t.state = match t.state {
                    TaskState::Running if waiting => TaskState::NeedsMe,
                    TaskState::NeedsMe if !waiting => TaskState::Running,
                    state => state,
                };
            }
            _ => continue,
        }
        shown.extend(task.clone());
    }
    shown
}

/// The steps the assignee sends for a mirrored task.
fn steps(task: &Task) -> Vec<TaskStep> {
    task.steps
        .iter()
        .map(|s| {
            let state = match s.state {
                StepState::Pending => TaskStatus::Proposed,
                StepState::Running => TaskStatus::Running,
                StepState::NeedsMe => {
                    return approval_step(s.id.as_str(), &s.label, &format!("grant_{GRANT}"));
                }
                StepState::Done => TaskStatus::Done,
                StepState::Failed => TaskStatus::Failed,
                StepState::Skipped => TaskStatus::Cancelled,
            };
            TaskStep { id: s.id.to_string(), title: s.label.clone(), state, needs_me: None }
        })
        .collect()
}

fn status(state: TaskState) -> TaskStatus {
    match state {
        TaskState::Proposed => TaskStatus::Proposed,
        TaskState::Running => TaskStatus::Running,
        TaskState::NeedsMe => TaskStatus::NeedsMe,
        TaskState::Done => TaskStatus::Done,
        TaskState::Failed => TaskStatus::Failed,
        TaskState::Cancelled => TaskStatus::Cancelled,
    }
}

/// One step as both paths must show it: id, label, state and (while it
/// waits) where it is decided.
type StepShape = (String, String, StepState, Option<NeedsMe>);

/// What both paths must agree on: version, state and the steps.
fn shape(task: &Task) -> (u64, TaskState, Vec<StepShape>) {
    let steps = task
        .steps
        .iter()
        .map(|s| {
            let place = (s.state == StepState::NeedsMe).then(|| s.needs_me.clone()).flatten();
            (s.id.to_string(), s.label.clone(), s.state, place)
        })
        .collect();
    (task.version, task.state, steps)
}

fn shown(store: &Store, id: &TaskId) -> Option<Task> {
    store.task(id).cloned()
}

#[test]
fn s10_replays_through_the_daemon_to_the_same_task() {
    let mirror = mirrored();
    assert_eq!(mirror.iter().map(|t| t.version).collect::<Vec<_>>(), (1..=10).collect::<Vec<_>>());
    let (world, path) = World::start("s10");
    let mut account = open(&path);
    let id = key(GROUP, TASK);
    for expected in &mirror {
        match expected.version {
            1 => {
                let plan: Vec<(&str, &str)> =
                    expected.steps.iter().map(|s| (s.id.as_str(), s.label.as_str())).collect();
                world.tasks.propose(GROUP, TASK, &expected.title, &plan).unwrap();
            }
            // The user starts the proposal.
            2 => act(&mut account, human(IntentKind::AcceptTaskProposal { task_id: id.clone() })),
            5 => {
                world.approvals.lock().unwrap().push(json!({
                    "approval_id": GRANT, "status": "pending", "server_id": "deploy",
                    "tool_name": "staging", "tool_scope": null, "risk_level": "high",
                    "expires_at": 1_990_000_000,
                    "details": { "arguments_hash": "sha256:5f1c09a2d7e4", "single_use": true,
                                 "requested_grant_id": format!("grant_{GRANT}") },
                }));
                world.fake.publish(
                    ACCOUNT,
                    "mcp.approval.request",
                    json!({ "approval_id": GRANT }),
                );
                account.until("the approval", |s, _| s.grant(&GrantId::new(GRANT)).is_some());
                world
                    .tasks
                    .update(GROUP, TASK, status(expected.state), steps(expected), None)
                    .unwrap();
            }
            // The user approves the deploy with a hold; the assignee checks
            // the grant and confirms the step.
            6 => {
                let mut intent = human(IntentKind::Approve { grant_id: GrantId::new(GRANT) });
                intent.confirmed_by = ConfirmationIssuer::new().issue_hold(1_500);
                act(&mut account, intent);
                account.until("the decision on its way", |s, _| {
                    shown(s, &id).is_some_and(|t| t.answer.is_some())
                });
                assert_eq!(world.tasks.confirm_decisions(), 1);
            }
            _ => {
                let result = expected.result_ref.as_ref().map(|r| r.as_str());
                world
                    .tasks
                    .update(GROUP, TASK, status(expected.state), steps(expected), result)
                    .unwrap();
            }
        }
        let version = expected.version;
        account.until(&format!("version {version}"), |s, _| {
            shown(s, &id).is_some_and(|t| t.version == version)
        });
        let task = shown(&account.store, &id).expect("task");
        assert_eq!(shape(&task), shape(expected), "at version {version}");
    }
    let done = shown(&account.store, &id).expect("task");
    assert_eq!(done.state, TaskState::Done);
    assert_eq!(done.result_ref, mirror.last().and_then(|t| t.result_ref.clone()));
    let resolve = world.since(0, "task.resolve").pop().expect("the deploy step was resolved");
    assert_eq!(
        (resolve.body["decision_id"].as_str(), resolve.body["base_revision"].as_u64()),
        (Some(decision_id(GRANT).as_str()), Some(5))
    );
    assert!(account.store.rejections().is_empty(), "{:?}", account.store.rejections());
    world.fake.shutdown();
}
