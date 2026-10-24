// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Tasks in the production window on the SDK source, against a fake daemon
//! that holds them on the protocol's state machine (see `sdk_fake`): asked
//! from the composer and cancelled from the card, a proposal started from
//! its card, an approval decided in its step with the hold and confirmed by
//! the assignee, a stalled assignee, a refusal shown on the card, and
//! someone else's task that this account does not decide. Each drives real
//! input and asserts what reached the daemon.

mod common;
mod sdk_fake;

use common::*;
use gpui_kit::TestAppContext;
use ramflux_desktop_core::{ContextId, GrantId, Task, TaskId, TaskState};
use ramflux_desktop_sdk::translate::tasks::{task_key, task_moment_id};
use ramflux_desktop_test::fake_bus::FakeBus;
use ramflux_desktop_test::fake_tasks::{FakeTasks, approval_step};
use ramflux_desktop_ui::route::Page;
use ramflux_desktop_ui::workspace::ComposerMode;
use ramflux_protocol::TaskStatus;
use sdk_fake::{BOT, GROUP, wait};

type Handle = gpui_kit::WindowHandle<gpui_kit::base::Root>;
type ShellEntity = gpui_kit::Entity<ramflux_desktop_ui::Shell>;

struct Opened {
    handle: Handle,
    shell: ShellEntity,
    fake: FakeBus,
    tasks: FakeTasks,
    state: sdk_fake::Shared,
}

fn open_group(cx: &mut TestAppContext, name: &str) -> Opened {
    let (fake, state, path, tasks) = sdk_fake::start_with_tasks(name);
    let (handle, shell) = open_with(cx, sdk_fake::options(&path));
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "the group", |cx| {
        with_store(cx, &shell, |s| s.context(&ContextId::new(GROUP)).is_some())
    });
    show_context(cx, handle, &shell, GROUP);
    Opened { handle, shell, fake, tasks, state }
}

fn task(cx: &mut TestAppContext, opened: &Opened, id: &TaskId) -> Option<Task> {
    with_store(cx, &opened.shell, |s| s.task(id).cloned())
}

fn scope(id: &TaskId) -> String {
    format!("moment-{}", task_moment_id(id))
}

fn sent(fake: &FakeBus, method: &str) -> Vec<serde_json::Value> {
    fake.seen().into_iter().filter(|s| s.method == method).map(|s| s.body).collect()
}

fn click_on(cx: &mut TestAppContext, opened: &Opened, id: &TaskId, element: &'static str) {
    let scope = scope(id);
    in_window(cx, opened.handle, move |window, cx| {
        gpui_kit::test::TestWindowExt::within(window, scope).click(element, cx);
    });
}

fn shown(cx: &mut TestAppContext, opened: &Opened, id: &TaskId, element: &str) -> bool {
    exists_in(cx, opened.handle, &scope(id), element)
}

#[gpui_kit::test]
fn a_task_asked_from_the_composer_runs_and_is_cancelled_from_its_card(cx: &mut TestAppContext) {
    let opened = open_group(cx, "ui-task-ask");
    click(cx, opened.handle, "composer-mode");
    let ws = workspace(cx, &opened.shell);
    let mode = cx.update(|cx| ws.read(cx).composer().read(cx).mode());
    assert_eq!(mode, ComposerMode::Task, "the group has an AI participant to ask");
    type_into(cx, opened.handle, "composer-input", "Sort out the open launch items");
    click(cx, opened.handle, "composer-send");
    wait(cx, "task.start", |_| !sent(&opened.fake, "task.start").is_empty());
    let start = sent(&opened.fake, "task.start").remove(0);
    assert_eq!(
        (start["conversation_id"].as_str(), start["assignee"].as_str()),
        (Some(GROUP), Some(BOT))
    );
    assert_eq!(start["text"], "Sort out the open launch items");
    let id = task_key(GROUP, start["task_id"].as_str().expect("task id"));
    wait(cx, "the card", |cx| task(cx, &opened, &id).is_some());
    advance(cx, 50);
    assert!(shown(cx, &opened, &id, "task-kind"), "the AI participant's kind is not verified");
    click_on(cx, &opened, &id, "task-cancel");
    wait(cx, "cancelled", |cx| {
        task(cx, &opened, &id).is_some_and(|t| t.state == TaskState::Cancelled)
    });
    assert_eq!(sent(&opened.fake, "task.cancel")[0]["version"], 1);
    advance(cx, 50);
    assert!(!shown(cx, &opened, &id, "task-cancel"), "a finished task has nothing to cancel");
    opened.fake.shutdown();
}

#[gpui_kit::test]
fn a_proposal_is_started_from_its_card(cx: &mut TestAppContext) {
    let opened = open_group(cx, "ui-task-proposal");
    opened.tasks.propose(GROUP, "t-plan", "Draft the release notes", &[("a", "Draft")]).unwrap();
    let id = task_key(GROUP, "t-plan");
    wait(cx, "the proposal", |cx| task(cx, &opened, &id).is_some());
    advance(cx, 50);
    assert!(shown(cx, &opened, &id, "task-start"));
    click_on(cx, &opened, &id, "task-start");
    wait(cx, "running", |cx| task(cx, &opened, &id).is_some_and(|t| t.state == TaskState::Running));
    let accept = sent(&opened.fake, "task.accept").remove(0);
    assert_eq!((accept["task_id"].as_str(), accept["version"].as_u64()), (Some("t-plan"), Some(2)));
    opened.fake.shutdown();
}

#[gpui_kit::test]
fn an_approval_is_decided_in_its_step_with_the_hold_then_confirmed(cx: &mut TestAppContext) {
    let opened = open_group(cx, "ui-task-approve");
    opened.tasks.start_as("principal-me", GROUP, "t-deploy", BOT, "Deploy to staging").unwrap();
    opened.state.lock().unwrap().approvals.push(sdk_fake::approval("appr-9", "high"));
    opened.fake.publish(
        "acct-main",
        "mcp.approval.request",
        serde_json::json!({ "approval_id": "appr-9" }),
    );
    wait(cx, "the approval", |cx| {
        with_store(cx, &opened.shell, |s| s.grant(&GrantId::new("appr-9")).is_some())
    });
    let steps = vec![approval_step("deploy", "Deploy to staging", "grant_appr-9")];
    opened.tasks.update(GROUP, "t-deploy", TaskStatus::NeedsMe, steps, None).unwrap();
    let id = task_key(GROUP, "t-deploy");
    wait(cx, "needs me", |cx| {
        task(cx, &opened, &id).is_some_and(|t| t.state == TaskState::NeedsMe)
    });
    advance(cx, 50);
    assert!(shown(cx, &opened, &id, "step-decision"), "decided in its step");
    click_on(cx, &opened, &id, "step-approve");
    assert!(strip_open(cx, &opened.shell), "a high-risk approval needs the hold");
    assert!(sent(&opened.fake, "grant.approve").is_empty(), "nothing before the hold");
    hold_confirm(cx, opened.handle);
    wait(cx, "on its way", |cx| task(cx, &opened, &id).is_some_and(|t| t.answer.is_some()));
    advance(cx, 50);
    assert!(shown(cx, &opened, &id, "task-answer"), "the card says the decision is on its way");
    assert_eq!(sent(&opened.fake, "task.resolve")[0]["decision_id"], "appr-9");
    assert_eq!(opened.tasks.confirm_decisions(), 1);
    wait(cx, "running", |cx| {
        task(cx, &opened, &id).is_some_and(|t| t.state == TaskState::Running && t.answer.is_none())
    });
    opened.fake.shutdown();
}

#[gpui_kit::test]
fn a_stalled_assignee_and_a_changed_task_are_shown_in_the_card(cx: &mut TestAppContext) {
    let opened = open_group(cx, "ui-task-stalled");
    opened.tasks.start_as("principal-me", GROUP, "t-index", BOT, "Index the archive").unwrap();
    let id = task_key(GROUP, "t-index");
    wait(cx, "the task", |cx| task(cx, &opened, &id).is_some());
    opened.tasks.stall(GROUP, "t-index");
    wait(cx, "stalled", |cx| task(cx, &opened, &id).is_some_and(|t| t.flags.stalled));
    advance(cx, 50);
    assert!(shown(cx, &opened, &id, "task-stalled"));
    assert!(shown(cx, &opened, &id, "task-cancel"), "the requester can still cancel");

    // Another device accepted a proposal a moment ago: the acceptance from
    // here is refused as changed, shown on the card.
    opened.tasks.propose(GROUP, "t-other", "Rotate the keys", &[]).unwrap();
    let other = task_key(GROUP, "t-other");
    wait(cx, "the proposal", |cx| task(cx, &opened, &other).is_some());
    opened.tasks.set_quiet(true);
    opened.tasks.accept_as("principal-ana", GROUP, "t-other").unwrap();
    opened.tasks.set_quiet(false);
    advance(cx, 50);
    click_on(cx, &opened, &other, "task-start");
    wait(cx, "Ana's task, to review", |cx| {
        task(cx, &opened, &other).is_some_and(|t| {
            t.state == TaskState::Running
                && t.answer
                    == Some(ramflux_desktop_core::TaskAnswer::Refused(
                        ramflux_desktop_core::TaskRefusal::Changed,
                    ))
        })
    });
    advance(cx, 50);
    assert!(shown(cx, &opened, &other, "task-answer"));
    assert!(shown(cx, &opened, &other, "task-asked-by"), "it is Ana's now");
    assert!(
        with_store(cx, &opened.shell, |s| s.failures().is_empty()),
        "on the card, not in the status strip"
    );
    opened.fake.shutdown();
}

#[gpui_kit::test]
fn someone_elses_task_says_who_decides_and_offers_nothing_to_do(cx: &mut TestAppContext) {
    let opened = open_group(cx, "ui-task-theirs");
    opened.tasks.start_as("principal-ana", GROUP, "t-ana", BOT, "Rotate keys").unwrap();
    opened.state.lock().unwrap().approvals.push(sdk_fake::approval("appr-4", "low"));
    opened.fake.publish(
        "acct-main",
        "mcp.approval.request",
        serde_json::json!({ "approval_id": "appr-4" }),
    );
    let steps = vec![approval_step("rotate", "Rotate keys", "grant_appr-4")];
    opened.tasks.update(GROUP, "t-ana", TaskStatus::NeedsMe, steps, None).unwrap();
    let id = task_key(GROUP, "t-ana");
    wait(cx, "needs Ana", |cx| {
        task(cx, &opened, &id).is_some_and(|t| t.state == TaskState::NeedsMe)
    });
    advance(cx, 50);
    assert!(shown(cx, &opened, &id, "step-not-mine"));
    assert!(!shown(cx, &opened, &id, "step-approve"), "Ana decides her step");
    assert!(!shown(cx, &opened, &id, "task-cancel"), "only Ana cancels it");
    assert!(shown(cx, &opened, &id, "task-asked-by"));
    opened.fake.shutdown();
}
