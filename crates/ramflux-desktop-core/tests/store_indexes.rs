// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The store's indexes agree with a full scan after every kind of change,
//! and message bodies live in the bounded cache that a lock clears.

mod common;

use common::*;
use ramflux_desktop_core::*;

fn intervention(grant: &str) -> MomentKind {
    MomentKind::Intervention(InterventionPayload {
        grant_id: GrantId::new(grant),
        risk: Risk::High,
        summary: format!("approve {grant}"),
    })
}

fn task_progress(task: &str) -> MomentKind {
    MomentKind::TaskProgress(TaskProgressPayload { task_id: TaskId::new(task) })
}

fn task(id: &str, ctx: &str, grants: &[&str], version: u64) -> Task {
    Task {
        id: TaskId::new(id),
        intent_id: None,
        context_id: ContextId::new(ctx),
        title: format!("Task {id}"),
        requester: Actor::Human,
        assignee: ParticipantId::new("p-agent"),
        steps: grants
            .iter()
            .map(|g| TaskStep {
                id: StepId::new(format!("s-{g}")),
                label: format!("step {g}"),
                state: StepState::Pending,
                needs_me: Some(NeedsMe::Approval { grant_id: GrantId::new(*g) }),
            })
            .collect(),
        state: TaskState::Running,
        version,
        result: None,
        result_ref: None,
        flags: TaskFlags::default(),
        actors: Default::default(),
        answer: None,
    }
}

fn naive_timeline(store: &Store, ctx: &str) -> Vec<MomentId> {
    let mut moments: Vec<&Moment> =
        store.moments().filter(|m| m.context_id.as_str() == ctx).collect();
    moments.sort_by(|a, b| (a.at, &a.id).cmp(&(b.at, &b.id)));
    moments.into_iter().map(|m| m.id.clone()).collect()
}

fn naive_attention(store: &Store) -> Vec<MomentId> {
    let mut items: Vec<(Timestamp, MomentId)> = store
        .moments()
        .filter(|m| m.lifecycle == Lifecycle::NeedsMe)
        .filter(|m| {
            !matches!(&m.kind, MomentKind::Intervention(p) if store.moments().any(|t| matches!(&t.kind,
                MomentKind::TaskProgress(tp) if store.task(&tp.task_id).is_some_and(|task| task
                    .steps.iter().any(|s| s.grant_id() == Some(&p.grant_id))))))
        })
        .map(|m| (store.needs_me_since(&m.id).unwrap_or(m.at), m.id.clone()))
        .collect();
    items.sort();
    items.into_iter().map(|(_, id)| id).collect()
}

fn check(store: &Store) {
    for ctx in ["c-1", "c-2", "c-3"] {
        let indexed: Vec<MomentId> =
            selectors::timeline(store, &ContextId::new(ctx)).iter().map(|m| m.id.clone()).collect();
        assert_eq!(indexed, naive_timeline(store, ctx), "timeline of {ctx}");
        let needs = store
            .moments()
            .filter(|m| m.context_id.as_str() == ctx && m.lifecycle == Lifecycle::NeedsMe)
            .count();
        assert_eq!(selectors::needs_me_count(store, &ContextId::new(ctx)), needs);
    }
    let attention: Vec<MomentId> =
        selectors::attention(store).into_iter().map(|i| i.moment_id).collect();
    assert_eq!(attention, naive_attention(store));
    for who in ["p-ada", "p-bo"] {
        let mut naive: Vec<ContextId> = store
            .contexts()
            .filter(|c| c.participants.contains(&ParticipantId::new(who)))
            .map(|c| c.id.clone())
            .collect();
        naive.sort();
        let indexed: Vec<ContextId> = selectors::contexts_with(store, &ParticipantId::new(who))
            .iter()
            .map(|c| c.id.clone())
            .collect();
        assert_eq!(indexed, naive, "contexts of {who}");
    }
}

#[test]
fn indexes_follow_adds_updates_patches_tasks_and_removals() {
    let mut feed = Feed::new();
    let mut members = context("c-1", false);
    members.participants = vec![ParticipantId::new("p-ada"), ParticipantId::new("p-bo")];
    feed.ok(1, DomainEvent::ContextUpserted(members));
    feed.ok(1, DomainEvent::ContextUpserted(context("c-2", true)));
    feed.ok(1, DomainEvent::ContextUpserted(context("c-3", false)));
    feed.ok(2, DomainEvent::GrantRequested(grant("g-1", Risk::High)));
    feed.ok(2, DomainEvent::GrantRequested(grant("g-2", Risk::High)));
    for n in 0..30 {
        let ctx = ["c-1", "c-2", "c-3"][n % 3];
        let lifecycle = if n % 4 == 0 { Lifecycle::NeedsMe } else { Lifecycle::Pending };
        let at = i64::try_from(100 - n).unwrap_or(0);
        feed.ok(10, added(moment(&format!("m-{n:02}"), ctx, at, message("x"), lifecycle)));
    }
    check(&feed.store);
    feed.ok(20, added(moment("m-ask", "c-2", 5, intervention("g-1"), Lifecycle::NeedsMe)));
    feed.ok(21, added(moment("m-ask2", "c-2", 6, intervention("g-2"), Lifecycle::NeedsMe)));
    check(&feed.store);
    assert_eq!(
        selectors::presenting_moment(&feed.store, &GrantId::new("g-2")).map(|m| m.id.clone()),
        Some(MomentId::new("m-ask2"))
    );
    feed.ok(22, DomainEvent::TaskUpserted(task("t-1", "c-3", &["g-1"], 1)));
    feed.ok(23, added(moment("m-task", "c-3", 7, task_progress("t-1"), Lifecycle::NeedsMe)));
    check(&feed.store);
    let owner = selectors::owning_task_step(&feed.store, &GrantId::new("g-1")).expect("owner");
    assert_eq!((owner.moment_id.as_str(), owner.step), ("m-task", 0));
    assert!(
        !selectors::attention(&feed.store).iter().any(|i| i.moment_id.as_str() == "m-ask"),
        "the task carries the attention"
    );
    feed.ok(24, DomainEvent::TaskUpserted(task("t-1", "c-3", &["g-2"], 2)));
    check(&feed.store);
    assert!(selectors::owning_task_step(&feed.store, &GrantId::new("g-1")).is_none());
    feed.ok(30, set_lifecycle("m-00", Lifecycle::Done));
    feed.ok(31, set_lifecycle("m-01", Lifecycle::NeedsMe));
    feed.ok(32, set_lifecycle("m-01", Lifecycle::InProgress));
    feed.ok(33, set_lifecycle("m-04", Lifecycle::Dismissed));
    check(&feed.store);
    assert_eq!(selectors::in_flight(&feed.store), [MomentId::new("m-task"), MomentId::new("m-01")]);
    let mut regrouped = context("c-1", false);
    regrouped.participants = vec![ParticipantId::new("p-bo")];
    feed.ok(40, DomainEvent::ContextUpserted(regrouped));
    check(&feed.store);
    feed.ok(41, DomainEvent::ContextRemoved { context_id: ContextId::new("c-2") });
    check(&feed.store);
    assert!(selectors::presenting_moment(&feed.store, &GrantId::new("g-2")).is_some(), "the task");
    assert!(feed.store.grant_presenters(&GrantId::new("g-1")).next().is_none(), "removed with c-2");
}

#[test]
fn message_bodies_live_in_the_bounded_cache_and_a_lock_drops_them() {
    let mut feed = Feed { store: Store::with_body_capacity(3), seq: 0 };
    feed.ok(
        1,
        DomainEvent::AccountUpdated(Account {
            id: AccountId::new("a-1"),
            label: "Me".to_owned(),
            kind: AccountKind::HumanInteractive,
            node_id: None,
            device_id: None,
            lock_state: LockState::Unlocked,
            self_participant_id: None,
            secure_sessions: ramflux_desktop_core::SecureSessions::Ready,
            networking: ramflux_desktop_core::Networking::Live,
        }),
    );
    feed.ok(1, DomainEvent::ContextUpserted(context("c-1", false)));
    for n in 0..5 {
        let text = format!("body {n}");
        feed.ok(2, added(moment(&format!("m-{n}"), "c-1", n, message(&text), Lifecycle::Done)));
    }
    let row = feed.store.moment(&MomentId::new("m-4")).cloned().expect("row");
    assert!(matches!(&row.kind, MomentKind::Message(p) if p.text.is_empty()), "rows are metadata");
    assert_eq!(feed.store.body(&MomentId::new("m-4")), Some("body 4"));
    assert_eq!(feed.store.bodies().len(), 3, "bounded");
    assert_eq!(feed.store.body(&MomentId::new("m-0")), None, "evicted");
    let reload = DomainEvent::MomentUpdated {
        moment_id: MomentId::new("m-0"),
        lifecycle: None,
        kind: Some(message("body 0")),
    };
    feed.ok(3, reload);
    assert_eq!(feed.store.body(&MomentId::new("m-0")), Some("body 0"), "a reload refills it");
    let lock = DomainEvent::AccountLockChanged {
        account_id: AccountId::new("a-1"),
        lock_state: LockState::Locked,
    };
    let changes = feed.ok(4, lock);
    assert!(changes.bodies_cleared);
    assert!(feed.store.bodies().is_empty(), "nothing readable stays after a lock");
    assert!(feed.store.moment(&MomentId::new("m-4")).is_some(), "metadata stays");
}
