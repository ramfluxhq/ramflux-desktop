// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Tasks in the production window on the SDK source, against a fake daemon
//! that holds them on the protocol's state machine (see `sdk_fake`): asked
//! from the composer and cancelled from the card, a proposal started from
//! its card, an approval decided in its step with the hold, not delivered
//! and sent again with Retry, then confirmed by the assignee, a stalled assignee, a refusal shown on the card, and
//! someone else's task that this account does not decide; what a member
//! did to a task is a line (never wire text), and a denied task says who
//! declined it. Each drives real
//! input and asserts what reached the daemon.

mod common;
mod sdk_fake;

use common::*;
use gpui_kit::TestAppContext;
use ramflux_desktop_core::{ContextId, GrantId, Task, TaskId, TaskState};
use ramflux_desktop_sdk::translate::tasks::{task_key, task_moment_id};
use ramflux_desktop_test::fake_bus::FakeBus;
use ramflux_desktop_test::fake_tasks::{FakeTasks, approval_step, choice_step};
use ramflux_desktop_ui::i18n::{tr, tr_args};
use ramflux_desktop_ui::route::Page;
use ramflux_desktop_ui::workspace::ComposerMode;
use ramflux_protocol::TaskStatus;
use sdk_fake::author::{Bot, scenario_surfaces};
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
    let resolve = &sent(&opened.fake, "task.resolve")[0];
    assert_eq!(resolve["decision_id"], "mcpd_appr-9", "the decision id the grant flow answered");

    // The decision did not reach the assignee: the card says so, and Retry
    // sends the same approval again (no second hold, no second resolution).
    opened.tasks.fail_delivery(GROUP, "t-deploy", "deploy", "appr-9");
    wait(cx, "not delivered", |cx| {
        task(cx, &opened, &id).is_some_and(|t| {
            matches!(t.answer, Some(ramflux_desktop_core::TaskAnswer::DecisionUndelivered { .. }))
        })
    });
    advance(cx, 50);
    assert!(shown(cx, &opened, &id, "task-retry-decision"));
    click_on(cx, &opened, &id, "task-retry-decision");
    wait(cx, "sent again", |_| sent(&opened.fake, "grant.approve").len() == 2);
    wait(cx, "on its way again", |cx| {
        task(cx, &opened, &id).is_some_and(|t| {
            matches!(t.answer, Some(ramflux_desktop_core::TaskAnswer::DecisionPending { .. }))
        })
    });
    assert!(!strip_open(cx, &opened.shell), "the decision was already confirmed");
    assert_eq!(sent(&opened.fake, "grant.approve")[1]["approval_id"], "appr-9");
    assert_eq!(sent(&opened.fake, "task.resolve").len(), 1);
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

/// Whether a task event line saying `text` is in the timeline.
fn line_says(cx: &mut TestAppContext, opened: &Opened, text: &str) -> bool {
    has_label(cx, opened.handle, text)
}

#[gpui_kit::test]
fn starting_a_proposal_is_a_line_in_the_timeline_not_wire_text(cx: &mut TestAppContext) {
    let opened = open_group(cx, "ui-task-event-line");
    opened.tasks.announce_messages(true);
    opened.tasks.propose(GROUP, "t-plan", "Draft the release notes", &[("a", "Draft")]).unwrap();
    let id = task_key(GROUP, "t-plan");
    wait(cx, "the proposal", |cx| task(cx, &opened, &id).is_some());
    advance(cx, 50);
    click_on(cx, &opened, &id, "task-start");
    wait(cx, "running", |cx| task(cx, &opened, &id).is_some_and(|t| t.state == TaskState::Running));
    // The acceptance's message is read with the conversation.
    show_context(cx, opened.handle, &opened.shell, GROUP);
    let you = tr("moment.task_event.you");
    let line = tr_args(
        "moment.task_event.accepted",
        &[("name", &you), ("object", "Draft the release notes")],
    );
    wait(cx, "the line", |cx| line_says(cx, &opened, &line));
    let bodies = with_store(cx, &opened.shell, |s| {
        s.context_moments(&ContextId::new(GROUP))
            .filter_map(|m| s.body(&m.id).map(str::to_owned))
            .collect::<Vec<_>>()
    });
    assert!(bodies.iter().all(|b| !b.contains("Signed event")), "{bodies:?}");
    opened.fake.shutdown();
}

#[gpui_kit::test]
fn a_denied_task_says_who_declined_it_instead_of_an_empty_summary(cx: &mut TestAppContext) {
    let opened = open_group(cx, "ui-task-denied");
    opened.tasks.announce_messages(true);
    opened.tasks.start_as("principal-me", GROUP, "t-prod", BOT, "Deploy to production").unwrap();
    opened.state.lock().unwrap().approvals.push(sdk_fake::approval("appr-4", "low"));
    opened.fake.publish(
        "acct-main",
        "mcp.approval.request",
        serde_json::json!({ "approval_id": "appr-4" }),
    );
    wait(cx, "the approval", |cx| {
        with_store(cx, &opened.shell, |s| s.grant(&GrantId::new("appr-4")).is_some())
    });
    let steps = vec![approval_step("deploy", "Deploy to staging", "grant_appr-4")];
    opened.tasks.update(GROUP, "t-prod", TaskStatus::NeedsMe, steps, None).unwrap();
    let id = task_key(GROUP, "t-prod");
    wait(cx, "needs me", |cx| {
        task(cx, &opened, &id).is_some_and(|t| t.state == TaskState::NeedsMe)
    });
    advance(cx, 50);
    click_on(cx, &opened, &id, "step-deny");
    wait(cx, "denied", |_| !sent(&opened.fake, "grant.deny").is_empty());
    wait(cx, "on its way", |cx| task(cx, &opened, &id).is_some_and(|t| t.answer.is_some()));
    assert_eq!(opened.tasks.confirm_decisions(), 1);
    // The denial ends the task as failed, without a result, as it did live.
    wait(cx, "failed", |cx| task(cx, &opened, &id).is_some_and(|t| t.state == TaskState::Failed));
    assert_eq!(task(cx, &opened, &id).and_then(|t| t.result), None, "no summary");
    show_context(cx, opened.handle, &opened.shell, GROUP);
    let declined =
        tr_args("moment.task.ending.declined_by", &[("name", &tr("moment.task.ending.you"))]);
    wait(cx, "who declined it", |cx| has_label(cx, opened.handle, &declined));
    assert!(shown(cx, &opened, &id, "task-ending"));
    let line = tr_args(
        "moment.task_event.denied",
        &[("name", &tr("moment.task_event.you")), ("object", "Deploy to staging")],
    );
    wait(cx, "the denial as a line", |cx| line_says(cx, &opened, &line));
    opened.fake.shutdown();
}

/// The daemon projects a task's message some time after the task changed,
/// so the desktop may read the failed task before the line that says who
/// declined it: the card must still say who once the line arrives.
#[gpui_kit::test]
fn a_denial_whose_line_arrives_after_the_failed_task_still_names_who_declined(
    cx: &mut TestAppContext,
) {
    let opened = open_group(cx, "ui-task-denied-late");
    opened.tasks.announce_messages(true);
    opened.tasks.hold_messages(true);
    opened.tasks.start_as("principal-me", GROUP, "t-late", BOT, "Deploy to production").unwrap();
    opened.state.lock().unwrap().approvals.push(sdk_fake::approval("appr-5", "low"));
    opened.fake.publish(
        "acct-main",
        "mcp.approval.request",
        serde_json::json!({ "approval_id": "appr-5" }),
    );
    wait(cx, "the approval", |cx| {
        with_store(cx, &opened.shell, |s| s.grant(&GrantId::new("appr-5")).is_some())
    });
    let steps = vec![approval_step("deploy", "Deploy to staging", "grant_appr-5")];
    opened.tasks.update(GROUP, "t-late", TaskStatus::NeedsMe, steps, None).unwrap();
    let id = task_key(GROUP, "t-late");
    wait(cx, "needs me", |cx| {
        task(cx, &opened, &id).is_some_and(|t| t.state == TaskState::NeedsMe)
    });
    advance(cx, 50);
    click_on(cx, &opened, &id, "step-deny");
    wait(cx, "on its way", |cx| task(cx, &opened, &id).is_some_and(|t| t.answer.is_some()));
    assert_eq!(opened.tasks.confirm_decisions(), 1);
    wait(cx, "failed", |cx| task(cx, &opened, &id).is_some_and(|t| t.state == TaskState::Failed));
    advance(cx, 500);
    let declined =
        tr_args("moment.task.ending.declined_by", &[("name", &tr("moment.task.ending.you"))]);
    assert!(!has_label(cx, opened.handle, &declined), "nothing says who yet");
    opened.tasks.hold_messages(false);
    wait(cx, "who declined it", |cx| has_label(cx, opened.handle, &declined));
    opened.fake.shutdown();
}

/// The daemon projects the surface that presents a task's result some time
/// after the task changed (here it is held back, then announced), so the
/// card is built without it first. Once the surface arrives, the card must
/// offer "Show result", read off the rendered card, not the model.
#[gpui_kit::test]
fn a_result_surface_that_arrives_after_its_task_card_adds_show_result(cx: &mut TestAppContext) {
    let opened = open_group(cx, "ui-task-result-late");
    opened
        .tasks
        .start_as("principal-me", GROUP, "t-res", BOT, "Prepare the release notes")
        .unwrap();
    let id = task_key(GROUP, "t-res");
    wait(cx, "the card", |cx| task(cx, &opened, &id).is_some());
    // The surface exists at the daemon but is not announced yet.
    // It is dated after the card and the lines of the task, so it lands at
    // the end of the timeline: appended, never a reason to rebuild the rows.
    opened.state.lock().unwrap().surfaces.set_author_clock(1_790_009_000);
    let mut bot = Bot::new();
    let event = scenario_surfaces("s10_ai_participant").remove(0);
    let (reference, accepted) = bot.send(&opened.fake, &opened.state, &event, false);
    assert!(accepted);
    opened
        .tasks
        .finish_with_surface(
            GROUP,
            "t-res",
            TaskStatus::Done,
            Vec::new(),
            (&reference, "Release notes drafted"),
        )
        .unwrap();
    wait(cx, "done", |cx| task(cx, &opened, &id).is_some_and(|t| t.state == TaskState::Done));
    advance(cx, 500);
    let surface = ramflux_desktop_core::SurfaceId::new(&reference);
    assert!(
        with_store(cx, &opened.shell, |s| s.surface_moment(&surface).is_none()),
        "the surface is still held back"
    );
    assert!(shown(cx, &opened, &id, "task-result"), "the summary is on the card");
    assert!(!shown(cx, &opened, &id, "task-show-result"), "nothing to point at yet");
    Bot::announce(&opened.fake, &opened.state, &reference);
    wait(cx, "the surface", |cx| {
        with_store(cx, &opened.shell, |s| s.surface_moment(&surface).is_some())
    });
    wait(cx, "the pointer on the card", |cx| shown(cx, &opened, &id, "task-show-result"));
    opened.fake.shutdown();
}

/// The same ordering for the surface a step asks its question in: the
/// assignee marks the step as needing a choice, and the surface that holds
/// the choice reaches the desktop afterwards. The card first says only that
/// the answer is wanted in the view; once the surface arrives it must offer
/// the way there ("Show view"), read off the rendered card.
#[gpui_kit::test]
fn a_decision_surface_that_arrives_after_its_task_card_adds_the_pointer(cx: &mut TestAppContext) {
    let opened = open_group(cx, "ui-task-decision-late");
    opened.tasks.start_as("principal-me", GROUP, "t-pick", BOT, "Pick the release day").unwrap();
    let id = task_key(GROUP, "t-pick");
    wait(cx, "the card", |cx| task(cx, &opened, &id).is_some());
    // Held back at the daemon and dated after the card, so it lands at the
    // end of the timeline: appended, never a reason to rebuild the rows.
    opened.state.lock().unwrap().surfaces.set_author_clock(1_790_009_000);
    let create = scenario_surfaces("s11_surface_versions").remove(0);
    let mut bot = Bot::new();
    let (reference, accepted) = bot.send(&opened.fake, &opened.state, &create, false);
    assert!(accepted);
    let steps = vec![choice_step("pick", "Pick the day", "s-pick")];
    opened.tasks.update(GROUP, "t-pick", TaskStatus::NeedsMe, steps, None).unwrap();
    wait(cx, "needs me", |cx| {
        task(cx, &opened, &id).is_some_and(|t| t.state == TaskState::NeedsMe)
    });
    advance(cx, 500);
    let surface = ramflux_desktop_core::SurfaceId::new(&reference);
    assert!(
        with_store(cx, &opened.shell, |s| s.surface_moment(&surface).is_none()),
        "the surface is still held back"
    );
    assert!(shown(cx, &opened, &id, "step-pointer"), "the step says the answer is in the view");
    assert!(!shown(cx, &opened, &id, "step-show-view"), "nothing to point at yet");
    Bot::announce(&opened.fake, &opened.state, &reference);
    wait(cx, "the surface", |cx| {
        with_store(cx, &opened.shell, |s| s.surface_moment(&surface).is_some())
    });
    wait(cx, "the pointer on the card", |cx| shown(cx, &opened, &id, "step-show-view"));
    opened.fake.shutdown();
}

#[gpui_kit::test]
fn a_peer_whose_kind_nothing_reported_is_not_asked_and_the_composer_says_why(
    cx: &mut TestAppContext,
) {
    // The recorded identity directory reports no kind for Ana (a remote
    // principal's kind is null unless a contact card asserted one).
    let opened = open_group(cx, "ui-task-kind-unknown");
    show_context(cx, opened.handle, &opened.shell, "dm-ana");
    let kind = with_store(cx, &opened.shell, |s| {
        s.participant(&ramflux_desktop_core::ParticipantId::new("principal-ana")).map(|p| p.kind)
    });
    assert_eq!(kind, Some(ramflux_desktop_core::ParticipantKind::Unknown));
    click(cx, opened.handle, "composer-mode");
    let ws = workspace(cx, &opened.shell);
    let composer = cx.update(|cx| ws.read(cx).composer().clone());
    assert_eq!(cx.update(|cx| composer.read(cx).mode()), ComposerMode::Message, "nobody to ask");
    let reason = cx.update(|cx| composer.read(cx).no_agent_reason(cx));
    assert_eq!(reason, tr_args("composer.kind_unknown", &[("name", "principal-ana")]));
    // The conversation is named after Ana, not by its id.
    assert!(exists(cx, opened.handle, "conversation-title"));
    let title = with_store(cx, &opened.shell, |s| {
        s.context(&ContextId::new("dm-ana"))
            .map(|c| ramflux_desktop_core::selectors::context_title(s, c))
    });
    assert_eq!(title.as_deref(), Some("principal-ana"));
    assert!(sent(&opened.fake, "task.start").is_empty());
    opened.fake.shutdown();
}
