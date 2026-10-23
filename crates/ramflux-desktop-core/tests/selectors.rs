// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Selectors over hand-built stores.

mod common;

use common::*;
use ramflux_desktop_core::selectors::{
    attention, contacts, context_stack, grants, in_flight, objects, participants, timeline,
};
use ramflux_desktop_core::*;

fn ids(list: &[ContextId]) -> Vec<&str> {
    list.iter().map(ContextId::as_str).collect()
}

#[test]
fn context_stack_puts_pinned_first_in_pin_order() {
    let mut feed = Feed::new();
    // Pinned out of id order: c-z first, then c-a.
    feed.ok(1, DomainEvent::ContextUpserted(context("c-z", true)));
    feed.ok(2, DomainEvent::ContextUpserted(context("c-a", true)));
    feed.ok(3, DomainEvent::ContextUpserted(context("c-m", false)));
    feed.ok(4, DomainEvent::ContextUpserted(context("c-busy", false)));
    // Needs-me in c-m; c-busy only has the most recent activity.
    feed.ok(5, added(moment("m-1", "c-m", 5, message("approve?"), Lifecycle::NeedsMe)));
    feed.ok(9, added(moment("m-2", "c-busy", 9, message("latest"), Lifecycle::Done)));

    let stack = context_stack(&feed.store);

    assert_eq!(ids(&stack), ["c-z", "c-a", "c-m"]);
}

#[test]
fn context_stack_deduplicates_pinned_contexts_that_need_me() {
    let mut feed = Feed::new();
    feed.ok(1, DomainEvent::ContextUpserted(context("c-1", true)));
    feed.ok(2, DomainEvent::ContextUpserted(context("c-2", false)));
    feed.ok(3, added(moment("m-1", "c-2", 3, message("a"), Lifecycle::NeedsMe)));
    feed.ok(4, added(moment("m-2", "c-1", 4, message("b"), Lifecycle::NeedsMe)));
    feed.ok(5, added(moment("m-3", "c-2", 5, message("c"), Lifecycle::NeedsMe)));

    assert_eq!(ids(&context_stack(&feed.store)), ["c-1", "c-2"]);
}

#[test]
fn context_stack_orders_attention_contexts_by_oldest_need_and_caps_at_six() {
    let mut feed = Feed::new();
    feed.ok(1, DomainEvent::ContextUpserted(context("pin-1", true)));
    feed.ok(2, DomainEvent::ContextUpserted(context("pin-2", true)));
    for (i, name) in ["n-f", "n-e", "n-d", "n-c", "n-b", "n-a"].iter().enumerate() {
        let at = 10 + i as i64;
        feed.ok(at, DomainEvent::ContextUpserted(context(name, false)));
        let id = format!("m-{name}");
        feed.ok(at, added(moment(&id, name, at, message("x"), Lifecycle::NeedsMe)));
    }

    let stack = context_stack(&feed.store);

    assert_eq!(stack.len(), CONTEXT_STACK_CAP);
    assert_eq!(ids(&stack), ["pin-1", "pin-2", "n-f", "n-e", "n-d", "n-c"]);
}

#[test]
fn unpinning_removes_from_stack_and_repinning_moves_to_end() {
    let mut feed = Feed::new();
    feed.ok(1, DomainEvent::ContextUpserted(context("c-1", true)));
    feed.ok(2, DomainEvent::ContextUpserted(context("c-2", true)));
    feed.ok(3, DomainEvent::ContextUpserted(context("c-1", false)));
    assert_eq!(ids(&context_stack(&feed.store)), ["c-2"]);
    feed.ok(4, DomainEvent::ContextUpserted(context("c-1", true)));
    assert_eq!(ids(&context_stack(&feed.store)), ["c-2", "c-1"]);
}

#[test]
fn attention_lists_needs_me_oldest_first_with_reason() {
    let mut feed = Feed::new();
    feed.ok(1, DomainEvent::ContextUpserted(context("c-1", false)));
    feed.ok(2, added(moment("m-late", "c-1", 2, message("x"), Lifecycle::Pending)));
    feed.ok(
        3,
        added(moment(
            "m-grant",
            "c-1",
            3,
            MomentKind::Intervention(InterventionPayload {
                grant_id: GrantId::new("g-1"),
                risk: Risk::High,
                summary: "Delete files".to_owned(),
            }),
            Lifecycle::NeedsMe,
        )),
    );
    feed.ok(4, added(moment("m-done", "c-1", 4, message("y"), Lifecycle::Done)));
    // m-late entered NeedsMe later than m-grant even though it is older.
    feed.ok(8, set_lifecycle("m-late", Lifecycle::NeedsMe));

    let items = attention(&feed.store);

    let order: Vec<_> = items.iter().map(|i| i.moment_id.as_str()).collect();
    assert_eq!(order, ["m-grant", "m-late"]);
    assert_eq!(items[0].reason, AttentionReason::Approval);
    assert_eq!(items[0].since, Timestamp(3));
    assert_eq!(items[1].since, Timestamp(8));
    assert_eq!(items[1].reason, AttentionReason::Other);
}

#[test]
fn in_flight_covers_in_progress_moments_and_open_tasks() {
    let mut feed = Feed::new();
    feed.ok(1, DomainEvent::ContextUpserted(context("c-1", false)));
    let task = |id: &str, lifecycle| Task {
        id: TaskId::new(id),
        intent_id: None,
        context_id: ContextId::new("c-1"),
        actor: Actor::Human,
        title: id.to_owned(),
        steps: Vec::new(),
        lifecycle,
        result: None,
    };
    feed.ok(2, DomainEvent::TaskUpserted(task("t-open", Lifecycle::NeedsMe)));
    feed.ok(2, DomainEvent::TaskUpserted(task("t-done", Lifecycle::Done)));
    let progress =
        |id: &str| MomentKind::TaskProgress(TaskProgressPayload { task_id: TaskId::new(id) });
    feed.ok(3, added(moment("m-open", "c-1", 3, progress("t-open"), Lifecycle::NeedsMe)));
    feed.ok(4, added(moment("m-closed", "c-1", 4, progress("t-done"), Lifecycle::Done)));
    feed.ok(5, added(moment("m-run", "c-1", 5, surface("s-1"), Lifecycle::InProgress)));
    feed.ok(6, added(moment("m-idle", "c-1", 6, message("x"), Lifecycle::Pending)));

    let flying: Vec<_> = in_flight(&feed.store).into_iter().map(|m| m.to_string()).collect();

    assert_eq!(flying, ["m-open", "m-run"]);
}

#[test]
fn timeline_orders_by_time_then_id_within_one_context() {
    let mut feed = Feed::new();
    feed.ok(1, DomainEvent::ContextUpserted(context("c-1", false)));
    feed.ok(1, DomainEvent::ContextUpserted(context("c-2", false)));
    feed.ok(2, added(moment("m-b", "c-1", 50, message("b"), Lifecycle::Done)));
    feed.ok(3, added(moment("m-a", "c-1", 50, message("a"), Lifecycle::Done)));
    feed.ok(4, added(moment("m-0", "c-1", 10, message("0"), Lifecycle::Done)));
    feed.ok(5, added(moment("m-x", "c-2", 1, message("x"), Lifecycle::Done)));

    let order: Vec<_> =
        timeline(&feed.store, &ContextId::new("c-1")).iter().map(|m| m.id.to_string()).collect();

    assert_eq!(order, ["m-0", "m-a", "m-b"]);
}

#[test]
fn dock_selectors_filter_and_order() {
    let mut feed = Feed::new();
    feed.ok(
        1,
        DomainEvent::ParticipantUpserted(participant("p-2", ParticipantKind::Human, "Bea", true)),
    );
    feed.ok(
        1,
        DomainEvent::ParticipantUpserted(participant("p-1", ParticipantKind::Human, "Ari", true)),
    );
    feed.ok(
        1,
        DomainEvent::ParticipantUpserted(participant(
            "p-3",
            ParticipantKind::Agent,
            "Orbit",
            false,
        )),
    );
    let mut ctx = context("c-1", false);
    ctx.participants =
        vec![ParticipantId::new("p-3"), ParticipantId::new("p-2"), ParticipantId::new("p-gone")];
    feed.ok(2, DomainEvent::ContextUpserted(ctx));
    feed.ok(3, DomainEvent::GrantRequested(grant("g-1", Risk::Low)));
    feed.ok(3, DomainEvent::GrantRequested(grant("g-2", Risk::High)));
    feed.ok(
        4,
        DomainEvent::GrantResolved { grant_id: GrantId::new("g-1"), state: GrantState::Denied },
    );
    feed.ok(
        5,
        DomainEvent::ObjectUpserted(Object {
            id: ObjectId::new("o-1"),
            name: "notes.txt".to_owned(),
            size: 12,
            direction: Direction::Outbound,
            transfer: TransferState::Complete,
            saved_to: None,
            preview: None,
        }),
    );

    let contact_names: Vec<_> = contacts(&feed.store).iter().map(|p| p.display.as_str()).collect();
    assert_eq!(contact_names, ["Ari", "Bea"]);

    let members: Vec<_> = participants(&feed.store, &ContextId::new("c-1"))
        .iter()
        .map(|p| p.id.to_string())
        .collect();
    assert_eq!(members, ["p-3", "p-2"]);
    assert!(participants(&feed.store, &ContextId::new("c-none")).is_empty());

    let pending: Vec<_> =
        grants(&feed.store, GrantFilter::Pending).iter().map(|g| g.id.to_string()).collect();
    assert_eq!(pending, ["g-2"]);
    assert_eq!(grants(&feed.store, GrantFilter::All).len(), 2);

    assert_eq!(objects(&feed.store).len(), 1);
}

#[test]
fn self_participant_comes_only_from_the_account() {
    let mut feed = Feed::new();
    let mut ctx = context("c-1", false);
    ctx.participants = vec![ParticipantId::new("p-me"), ParticipantId::new("p-ana")];
    feed.ok(
        1,
        DomainEvent::ParticipantUpserted(participant("p-me", ParticipantKind::Human, "Me", false)),
    );
    feed.ok(1, DomainEvent::ContextUpserted(ctx));
    assert_eq!(selectors::self_participant(&feed.store), None, "never guessed from membership");
    feed.ok(
        2,
        DomainEvent::AccountUpdated(Account {
            id: AccountId::new("a-1"),
            label: "Me".into(),
            kind: AccountKind::HumanInteractive,
            node_id: None,
            device_id: None,
            lock_state: LockState::Unlocked,
            self_participant_id: Some(ParticipantId::new("p-me")),
        }),
    );
    assert_eq!(selectors::self_participant(&feed.store), Some(ParticipantId::new("p-me")));
}
