// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The K3b r4 task contract on the SDK source (`docs/conversation_tasks.md`
//! of the product): refusals are read from the daemon's typed
//! `error.code`, never from its text.

mod common;

use common::task_world::*;
use ramflux_desktop_core::Source as _;
use ramflux_desktop_core::{Effect, TaskAnswer, TaskRefusal, methods};
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
