// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The K3b r4 task contract on the SDK source (`docs/conversation_tasks.md`
//! of the product): refusals are read from the daemon's typed
//! `error.code`, never from its text; a finished task shows its signed
//! summary; the assignee's own kind marker is shown; a pending resolution
//! is read back from the daemon after a restart and says when the assignee
//! has not confirmed it in time; compaction is announced; an approval is
//! resolved with the grant flow's decision id; a choice answered in the
//! assignee's surface is resolved with the action's verified receipt id.

mod common;

use common::task_world::*;
use ramflux_desktop_core::Source as _;
use ramflux_desktop_core::{
    ConfirmationIssuer, DomainEvent, Effect, GrantId, IntentKind, StepId, SurfaceEvent, SurfaceId,
    TaskAnswer, TaskRefusal, TaskState, methods,
};
use ramflux_desktop_sdk::surface::author::Author;
use ramflux_desktop_sdk::surface::surface_ref;
use ramflux_desktop_test::fake_tasks::{approval_step, choice_step, task_step};
use ramflux_protocol::TaskStatus;
use serde_json::json;

#[test]
fn refusals_come_from_typed_codes_whatever_the_text_says() {
    let (world, path) = World::start("codes");
    let mut account = open(&path);
    world.tasks.start_as("principal-me", GROUP, "t-1", BOT, "Index the archive").unwrap();
    let id = key(GROUP, "t-1");
    account.until("the task", |s, _| s.task(&id).is_some());
    // The text would read as "changed" or "not allowed" if it were parsed.
    let cases = [
        ("task_stalled", TaskRefusal::Stalled),
        ("quota_exhausted", TaskRefusal::Busy),
        ("not_requester", TaskRefusal::NotAllowed),
        ("rate_limited", TaskRefusal::Busy),
        ("terminal", TaskRefusal::Finished),
    ];
    for (code, expected) in cases {
        world.tasks.refuse_next("task.cancel", code, "expected task revision 9; requester only");
        account.source.submit(Effect::Call {
            method: methods::TASK_CANCEL.into(),
            body: json!({ "task_id": id, "version": 1, "decline": false }),
        });
        let answer = Some(TaskAnswer::Refused(expected.clone()));
        account.until(code, |s, _| s.task(&id).is_some_and(|t| t.answer == answer));
    }
    assert!(account.store.failures().is_empty(), "answered on the task, not the status strip");
    world.fake.shutdown();
}

#[test]
fn a_proposal_id_is_not_an_acceptance() {
    let (world, _path) = World::start("no-shim");
    world.tasks.propose(GROUP, "t-1", "Check the release", &[]).unwrap();
    let reply = world.tasks.handle(
        "task.start",
        &json!({ "conversation_id": GROUP, "proposal_id": "t-1", "version": 2 }),
    );
    assert!(
        matches!(reply, Some(ramflux_desktop_test::fake_bus::Reply::Error("invalid_request", _))),
        "acceptance is task.accept {{task_id, version}} only"
    );
    world.fake.shutdown();
}

#[test]
fn a_finished_task_shows_its_signed_summary_and_the_assignees_own_kind() {
    let (world, path) = World::start("summary");
    let mut account = open(&path);
    world.tasks.verify_kind(BOT);
    world.tasks.start_as("principal-me", GROUP, "t-1", BOT, "Inspect the logs").unwrap();
    let id = key(GROUP, "t-1");
    account.until("the task", |s, _| s.task(&id).is_some());
    // The view's own marker stays false; the assignee's is what is shown.
    assert_eq!(world.tasks.view(GROUP, "t-1")["actor_kind_verified"], false);
    assert!(account.store.task(&id).is_some_and(|t| t.flags.kind_verified));
    assert_eq!(account.store.task(&id).and_then(|t| t.result.clone()), None);

    let done = vec![task_step("a", "Inspect", TaskStatus::Done)];
    world.tasks.finish(GROUP, "t-1", TaskStatus::Done, done, "Inspection complete").unwrap();
    account.until("done with its summary", |s, _| {
        s.task(&id).is_some_and(|t| {
            t.state == TaskState::Done && t.result.as_deref() == Some("Inspection complete")
        })
    });
    // Compaction keeps the summary and is announced: no reopen needed.
    world.tasks.compact(GROUP, "t-1");
    account.until("compacted", |s, _| s.task(&id).is_some_and(|t| t.flags.compacted));
    assert_eq!(
        account.store.task(&id).and_then(|t| t.result.clone()).as_deref(),
        Some("Inspection complete")
    );
    assert!(account.store.rejections().is_empty(), "{:?}", account.store.rejections());
    world.fake.shutdown();
}

#[test]
fn a_pending_resolution_is_read_back_after_a_restart_and_says_when_it_waits_too_long() {
    let (world, path) = World::start("pending");
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
    drop(account);

    // A new desktop session reads the daemon's durable pending resolution.
    let mut account = open(&path);
    account.until("still pending", |s, _| s.task(&id).is_some_and(|t| t.answer == pending));
    world.tasks.time_out_decisions();
    let unconfirmed = Some(TaskAnswer::DecisionUnconfirmed { step_id: step });
    account.until("unconfirmed", |s, _| s.task(&id).is_some_and(|t| t.answer == unconfirmed));

    // The assignee can still commit it: the next version clears the notice.
    assert_eq!(world.tasks.confirm_decisions(), 1);
    account.until("running", |s, _| {
        s.task(&id).is_some_and(|t| t.state == TaskState::Running && t.answer.is_none())
    });
    world.fake.shutdown();
}

/// The assignee posts its surface `s-pick` (the first surface of s11) in
/// the group; returns the desktop's id of it.
fn post_pick_surface(world: &World) -> String {
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
    world.fake.publish(ACCOUNT, "surface.changed", world.surfaces.meta(&reference));
    reference
}

#[test]
fn a_choice_is_answered_in_the_surface_then_resolved_with_its_verified_receipt() {
    let (world, path) = World::start("choice");
    let mut account = open(&path);
    let reference = post_pick_surface(&world);
    assert_eq!(Some(reference.clone()), surface_ref(GROUP, BOT, "s-pick"));
    world.tasks.start_as("principal-me", GROUP, "t-day", BOT, "Pick the release day").unwrap();
    let steps = vec![choice_step("pick", "Pick the day", "s-pick")];
    world.tasks.update(GROUP, "t-day", TaskStatus::NeedsMe, steps, None).unwrap();
    let id = key(GROUP, "t-day");
    account.until("needs me in the surface", |s, _| {
        s.task(&id).is_some_and(|t| t.state == TaskState::NeedsMe)
            && s.surface_moment(&SurfaceId::new(&reference)).is_some()
    });
    let place = account.store.task(&id).and_then(|t| t.steps[0].needs_me.clone());
    assert_eq!(
        place.as_ref().and_then(|p| p.surface()).map(SurfaceId::as_str),
        Some(reference.as_str()),
        "the step names the assignee's surface by its scoped reference"
    );

    // The user answers in the surface.
    let (version, hash) = world.surfaces.current(&reference).expect("held");
    account.source.submit(Effect::Call {
        method: methods::SURFACE_ACTION.into(),
        body: json!({ "surface_id": reference, "version": version, "surface_hash": hash,
                      "action_id": "thu", "values": { "decision": "resume" } }),
    });
    account.until("the action", |_, _| world.surfaces.acted().len() == 1);
    account.settle(300);
    assert!(world.since(0, "task.resolve").is_empty(), "nothing before the author's receipt");

    // The receipt arrives: the step is resolved with its id, once.
    world.surfaces.deliver_receipts("principal-me");
    world.fake.publish(ACCOUNT, "surface.changed", world.surfaces.meta(&reference));
    let step = StepId::new("pick");
    let pending = Some(TaskAnswer::DecisionPending { step_id: step });
    account.until("pending", |s, _| s.task(&id).is_some_and(|t| t.answer == pending));
    let resolves = world.since(0, "task.resolve");
    assert_eq!(resolves.len(), 1);
    let resolve = &resolves[0].body;
    assert_eq!(resolve["decision_id"], "rcpt-1", "the action receipt id, not the action name");
    assert_eq!(resolve["decision"], "resume");
    assert_eq!(resolve["step_id"], "pick");
    assert_eq!(resolve["target"], json!({ "kind": "choice", "surface_id": "s-pick" }));
    assert_eq!(resolve["base_revision"], world.tasks.version(GROUP, "t-day"));
    // Another change of the surface resolves nothing more.
    world.fake.publish(ACCOUNT, "surface.changed", world.surfaces.meta(&reference));
    account.settle(300);
    assert_eq!(world.since(0, "task.resolve").len(), 1);
    assert!(account.store.failures().is_empty(), "{:?}", account.store.failures());
    world.fake.shutdown();
}
