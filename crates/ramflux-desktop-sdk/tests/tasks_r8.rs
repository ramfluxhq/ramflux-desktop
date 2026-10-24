// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The K3b r5–r8 task contract on the SDK source (`docs/conversation_tasks.md`
//! of the product at 6fcf0e9): a pruned task (`task.changed` `kind:
//! removed`) is dropped; a decision on an approval whose delivery to the
//! assignee failed is shown as not delivered, and Retry sends the same
//! `grant.approve` again (nothing else); a pending resolution names its
//! step; a restarted requester resolves a choice from the action's signed
//! task binding alone.

mod common;

use common::task_world::*;
use ramflux_desktop_core::Source as _;
use ramflux_desktop_core::{
    ConfirmationIssuer, DomainEvent, Effect, GrantId, IntentKind, MomentId, StepId, SurfaceEvent,
    SurfaceId, TaskAnswer, TaskState, methods,
};
use ramflux_desktop_sdk::surface::author::Author;
use ramflux_desktop_test::fake_tasks::{approval_step, choice_step, task_step};
use ramflux_protocol::TaskStatus;
use serde_json::json;

#[test]
fn a_pruned_task_is_dropped_with_its_moment_and_not_read_again() {
    let (world, path) = World::start("removed");
    let mut account = open(&path);
    world.tasks.start_as("principal-me", GROUP, "t-1", BOT, "Index the archive").unwrap();
    let id = key(GROUP, "t-1");
    let moment = MomentId::new(format!("task:{id}"));
    account.until("the task", |s, _| s.task(&id).is_some() && s.moment(&moment).is_some());
    let done = vec![task_step("a", "Index", TaskStatus::Done)];
    world.tasks.finish(GROUP, "t-1", TaskStatus::Done, done, "Indexed").unwrap();
    account.until("done", |s, _| s.task(&id).is_some_and(|t| t.state == TaskState::Done));

    let mark = world.mark();
    world.tasks.remove(GROUP, "t-1");
    account.until("dropped", |s, _| s.task(&id).is_none() && s.moment(&moment).is_none());
    account.settle(300);
    assert!(world.since(mark, "task.get").is_empty(), "a removal is not read");
    assert!(account.store.rejections().is_empty(), "{:?}", account.store.rejections());
    world.fake.shutdown();
}

#[test]
fn an_undelivered_decision_is_shown_and_retry_sends_the_same_decision_again() {
    let (world, path) = World::start("undelivered");
    let mut account = open(&path);
    world.tasks.start_as("principal-me", GROUP, "t-1", BOT, "Deploy to staging").unwrap();
    world.approvals.lock().unwrap().push(approval("appr-7", "high"));
    world.fake.publish(ACCOUNT, "mcp.approval.request", json!({ "approval_id": "appr-7" }));
    account.until("the approval", |s, _| s.grant(&GrantId::new("appr-7")).is_some());
    let steps = vec![approval_step("deploy", "Deploy to staging", "grant_appr-7")];
    world.tasks.update(GROUP, "t-1", TaskStatus::NeedsMe, steps, None).unwrap();
    let id = key(GROUP, "t-1");
    account.until("needs me", |s, _| s.task(&id).is_some_and(|t| t.state == TaskState::NeedsMe));
    let mut intent = human(IntentKind::Approve { grant_id: GrantId::new("appr-7") });
    intent.confirmed_by = ConfirmationIssuer::new().issue_hold(1_200);
    act(&mut account, intent);
    let step = StepId::new("deploy");
    let pending = Some(TaskAnswer::DecisionPending { step_id: step.clone() });
    account.until("pending", |s, _| s.task(&id).is_some_and(|t| t.answer == pending));

    // The courier ran out of retries: the step says so, with Retry.
    world.tasks.fail_delivery(GROUP, "t-1", "deploy", "appr-7");
    let undelivered = Some(TaskAnswer::DecisionUndelivered {
        step_id: step.clone(),
        grant_id: GrantId::new("appr-7"),
        approved: true,
    });
    account.until("not delivered", |s, _| s.task(&id).is_some_and(|t| t.answer == undelivered));

    // Retry: the same grant.approve, no new consent, no second resolution.
    let mark = world.mark();
    act(&mut account, human(IntentKind::RetryTaskDecision { task_id: id.clone() }));
    account.until("on its way again", |s, _| s.task(&id).is_some_and(|t| t.answer == pending));
    account.settle(300);
    let approves = world.since(mark, "grant.approve");
    assert_eq!(approves.len(), 1);
    assert_eq!(approves[0].body, json!({ "approval_id": "appr-7" }), "the same decision");
    assert!(world.since(mark, "grant.deny").is_empty());
    assert!(world.since(mark, "task.resolve").is_empty(), "the step was already resolved");
    assert!(account.store.failures().is_empty(), "{:?}", account.store.failures());

    // Nothing waits to be retried any more.
    let again = ramflux_desktop_core::handle_intent(
        &account.store,
        human(IntentKind::RetryTaskDecision { task_id: id.clone() }),
        ramflux_desktop_core::Timestamp(0),
    );
    assert!(again.is_err(), "no undelivered decision left");
    world.fake.shutdown();
}

#[test]
fn a_pending_resolution_names_its_step() {
    let (world, path) = World::start("pending-step");
    let mut account = open(&path);
    world.tasks.start_as("principal-me", GROUP, "t-1", BOT, "Deploy twice").unwrap();
    world.approvals.lock().unwrap().push(approval("appr-1", "low"));
    world.fake.publish(ACCOUNT, "mcp.approval.request", json!({ "approval_id": "appr-1" }));
    account.until("the approval", |s, _| s.grant(&GrantId::new("appr-1")).is_some());
    let steps = vec![
        task_step("build", "Build", TaskStatus::Done),
        approval_step("deploy", "Deploy", "grant_appr-1"),
    ];
    world.tasks.update(GROUP, "t-1", TaskStatus::NeedsMe, steps, None).unwrap();
    let id = key(GROUP, "t-1");
    account.until("needs me", |s, _| s.task(&id).is_some_and(|t| t.state == TaskState::NeedsMe));
    act(&mut account, human(IntentKind::Approve { grant_id: GrantId::new("appr-1") }));
    let pending = Some(TaskAnswer::DecisionPending { step_id: StepId::new("deploy") });
    account.until("pending", |s, _| s.task(&id).is_some_and(|t| t.answer == pending));
    assert_eq!(world.tasks.view(GROUP, "t-1")["pending_resolution"]["step_id"], "deploy");
    drop(account);
    // Read back after a restart, on the step the daemon names.
    let mut account = open(&path);
    account.until("still pending", |s, _| s.task(&id).is_some_and(|t| t.answer == pending));
    world.fake.shutdown();
}

#[test]
fn a_restarted_requester_resolves_a_choice_from_the_signed_binding_alone() {
    let (world, path) = World::start("restart-choice");
    let scenario = ramflux_desktop_test::load_fixture("s11_surface_versions").expect("fixture");
    let create = scenario
        .timeline
        .iter()
        .find_map(|(_, e)| match e {
            DomainEvent::Surface(event @ SurfaceEvent::Create { .. }) => Some(event.clone()),
            _ => None,
        })
        .expect("a create");
    let mut author = Author::default();
    let mut reference = None;
    author
        .send(GROUP, &create, |bytes| {
            reference = world.surfaces.author_event(BOT, bytes).ok();
            reference.is_some()
        })
        .expect("authored");
    let reference = reference.expect("accepted");
    world.tasks.start_as("principal-me", GROUP, "t-day", BOT, "Pick the release day").unwrap();
    let steps = vec![choice_step("pick", "Pick the day", "s-pick")];
    world.tasks.update(GROUP, "t-day", TaskStatus::NeedsMe, steps, None).unwrap();
    let id = key(GROUP, "t-day");
    // The daemon refuses the first resolution, as if the desktop stopped
    // before it went out: only the signed action is left.
    world.tasks.refuse_next("task.resolve", "Unavailable", "QUIC deadline");
    let mut account = open(&path);
    world.fake.publish(ACCOUNT, "surface.changed", world.surfaces.meta(&reference));
    account.until("needs me in the surface", |s, _| {
        s.task(&id).is_some_and(|t| t.state == TaskState::NeedsMe)
            && s.surface_moment(&SurfaceId::new(&reference)).is_some()
    });
    let (version, hash) = world.surfaces.current(&reference).expect("held");
    account.source.submit(Effect::Call {
        method: methods::SURFACE_ACTION.into(),
        body: json!({ "surface_id": reference, "version": version, "surface_hash": hash,
                      "action_id": "thu", "values": { "decision": "deny" } }),
    });
    account.until("refused, retryable", |s, _| {
        s.task(&id).is_some_and(|t| {
            t.answer == Some(TaskAnswer::Refused(ramflux_desktop_core::TaskRefusal::Unreachable))
        })
    });
    drop(account);

    // A new session knows nothing of the action; the read lists it with
    // its signed binding, which names the step: resolved with it.
    let mark = world.mark();
    let mut account = open(&path);
    world.fake.publish(ACCOUNT, "surface.changed", world.surfaces.meta(&reference));
    let pending = Some(TaskAnswer::DecisionPending { step_id: StepId::new("pick") });
    account.until("resolved after the restart", |s, _| {
        s.task(&id).is_some_and(|t| t.answer == pending)
    });
    let resolves = world.since(mark, "task.resolve");
    assert_eq!(resolves.len(), 1);
    assert_eq!(resolves[0].body["decision_id"], "rcpt-1");
    assert_eq!(resolves[0].body["decision"], "deny");
    assert_eq!(resolves[0].body["step_id"], "pick");
    world.fake.shutdown();
}
