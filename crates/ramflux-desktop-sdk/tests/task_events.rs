// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Task event messages on the SDK source (DESK-F17), against the fake
//! daemon whose task messages carry the protocol's own text rendering of
//! each signed event: an acceptance, a cancel and a step's decision become
//! compact task event lines by the member who signed them (never message
//! text); a sibling device's copy of this account's own event, listed
//! without task fields as a live daemon lists it, is the same line; a
//! peer's message that only looks like one stays that peer's message; and
//! a denied task says who denied it. The fake announces each task message
//! as `message.received`, as the daemon announces every projected message.

mod common;

use common::task_world::*;
use ramflux_desktop_core::Source as _;
use ramflux_desktop_core::{
    ContextId, GrantId, IntentKind, Moment, MomentKind, ParticipantId, StepId, Store,
    TaskEventKind, TaskState, selectors,
};
use ramflux_desktop_test::fake_tasks::approval_step;
use ramflux_protocol::TaskStatus;
use serde_json::json;

const DM: &str = "dm-ana";

fn lines(store: &Store, context: &str) -> Vec<(ParticipantId, TaskEventKind)> {
    store
        .context_moments(&ContextId::new(context))
        .filter_map(|m| match &m.kind {
            MomentKind::TaskEvent(p) => Some((m.origin.clone(), p.event.clone())),
            _ => None,
        })
        .collect()
}

fn texts(store: &Store, context: &str) -> Vec<String> {
    store
        .context_moments(&ContextId::new(context))
        .filter(|m: &&Moment| matches!(m.kind, MomentKind::Message(_)))
        .filter_map(|m| store.body(&m.id).map(str::to_owned))
        .collect()
}

#[test]
fn accepting_and_cancelling_are_lines_by_who_signed_them_never_message_text() {
    let (world, path) = World::start("events-accept");
    world.tasks.announce_messages(true);
    let mut account = open(&path);
    world.tasks.propose(GROUP, "t-1", "Check the release", &[("a", "Read")]).unwrap();
    let id = key(GROUP, "t-1");
    account.until("the proposal", |s, _| s.task(&id).is_some());
    act(&mut account, human(IntentKind::AcceptTaskProposal { task_id: id.clone() }));
    account.until("accepted, as a line", |s, _| {
        lines(s, GROUP).iter().any(|(_, e)| *e == TaskEventKind::Accepted)
    });
    act(&mut account, human(IntentKind::CancelTask { task_id: id.clone() }));
    account.until("cancelled, as a line", |s, _| {
        lines(s, GROUP).iter().any(|(_, e)| *e == TaskEventKind::Cancelled)
    });
    let shown = lines(&account.store, GROUP);
    assert!(
        shown.iter().all(|(who, _)| who.as_str() == "principal-me"),
        "each line is by the member who signed it: {shown:?}"
    );
    assert_eq!(shown.len(), 2, "the proposal is the card, not a line: {shown:?}");
    assert!(texts(&account.store, GROUP).iter().all(|t| !t.contains("Signed event")));
    let task = account.store.task(&id).cloned().expect("task");
    assert_eq!(task.state, TaskState::Cancelled);
    let ending = selectors::task_ending(&account.store, &task).expect("ending");
    assert_eq!(ending.how, selectors::TaskEndingKind::Cancelled);
    assert_eq!(ending.by.as_str(), "principal-me");
    world.fake.shutdown();
}

#[test]
fn a_sibling_copy_of_this_accounts_event_is_its_line_and_a_lookalike_stays_a_message() {
    let (world, path) = World::start("events-sibling");
    world.tasks.announce_messages(true);
    world.tasks.propose(DM, "t-9", "Deploy build 2.4.0-rc1", &[("a", "Deploy")]).unwrap();
    // The account's other device accepted it; this device lists that event
    // the way a live daemon lists a sibling's copy: no task fields.
    world.tasks.accept_as("principal-me", DM, "t-9").unwrap();
    world.tasks.as_sibling_copy(DM);
    let copy = world.tasks.messages(DM).pop().expect("copy");
    assert!(copy.get("content_type").is_none() && copy.get("task_id").is_none());
    let rendering = copy["body_utf8"].as_str().expect("text").to_owned();
    assert!(rendering.starts_with("Signed event signer="), "{rendering}");
    // Ana writes the same text: a peer's message is never read as an event.
    world.tasks.push_message(DM, "principal-ana", &rendering);

    let mut account = open(&path);
    account.source.focus(Some(&ContextId::new(DM)));
    let id = key(DM, "t-9");
    account.until("the task, read by the id the copy names", |s, _| {
        s.task(&id).is_some_and(|t| t.state == TaskState::Running)
    });
    account.until("the copy as a line", |s, _| !lines(s, DM).is_empty());
    assert_eq!(
        lines(&account.store, DM),
        vec![(ParticipantId::new("principal-me"), TaskEventKind::Accepted)]
    );
    let messages: Vec<(ParticipantId, String)> = account
        .store
        .context_moments(&ContextId::new(DM))
        .filter(|m| matches!(m.kind, MomentKind::Message(_)))
        .filter_map(|m| account.store.body(&m.id).map(|b| (m.origin.clone(), b.to_owned())))
        .filter(|(_, b)| b.starts_with("Signed event"))
        .collect();
    assert_eq!(
        messages,
        vec![(ParticipantId::new("principal-ana"), rendering)],
        "only Ana's own message shows that text"
    );
    world.fake.shutdown();
}

#[test]
fn a_denied_step_is_a_line_and_the_failed_task_says_who_declined_it() {
    let (world, path) = World::start("events-deny");
    world.tasks.announce_messages(true);
    let mut account = open(&path);
    world.tasks.start_as("principal-me", GROUP, "t-1", BOT, "Deploy to production").unwrap();
    world.approvals.lock().unwrap().push(approval("appr-1", "low"));
    world.fake.publish(ACCOUNT, "mcp.approval.request", json!({ "approval_id": "appr-1" }));
    account.until("the approval", |s, _| s.grant(&GrantId::new("appr-1")).is_some());
    let steps = vec![approval_step("deploy", "Deploy to staging", "grant_appr-1")];
    world.tasks.update(GROUP, "t-1", TaskStatus::NeedsMe, steps, None).unwrap();
    let id = key(GROUP, "t-1");
    account.until("needs me", |s, _| s.task(&id).is_some_and(|t| t.state == TaskState::NeedsMe));
    act(&mut account, human(IntentKind::Deny { grant_id: GrantId::new("appr-1") }));
    account.until("the decision pending", |s, _| s.task(&id).is_some_and(|t| t.answer.is_some()));
    world.tasks.confirm_decisions();
    // The denial ends the task as failed, without a result, as it did live.
    account.until("failed, with the denial as a line", |s, _| {
        s.task(&id).is_some_and(|t| t.state == TaskState::Failed)
            && lines(s, GROUP).iter().any(|(_, e)| matches!(e, TaskEventKind::Denied { .. }))
    });
    let shown = lines(&account.store, GROUP);
    assert!(
        shown.contains(&(
            ParticipantId::new("principal-me"),
            TaskEventKind::Denied { step_id: Some(StepId::new("deploy")) }
        )),
        "{shown:?}"
    );
    let task = account.store.task(&id).cloned().expect("task");
    assert_eq!(task.result, None, "no summary");
    let ending = selectors::task_ending(&account.store, &task).expect("ending");
    assert_eq!(ending.how, selectors::TaskEndingKind::Denied);
    assert_eq!(ending.by.as_str(), "principal-me");
    world.fake.shutdown();
}
