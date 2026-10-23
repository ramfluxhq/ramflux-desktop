// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Reducer behavior of the single-writer store.

mod common;

use common::*;
use ramflux_desktop_core::*;
use serde_json::json;

fn seeded() -> Feed {
    let mut feed = Feed::new();
    feed.ok(10, DomainEvent::ContextUpserted(context("c-1", false)));
    feed.ok(20, added(moment("m-1", "c-1", 20, message("hi"), Lifecycle::Pending)));
    feed
}

#[test]
fn schema_mismatch_is_rejected_without_consuming_seq() {
    let mut store = Store::new();
    let mut env =
        EventEnvelope::new(1, Timestamp(1), DomainEvent::ContextUpserted(context("c-1", false)));
    env.schema = "ramflux.desktop.event.v0".to_owned();

    let result = store.apply(&env);

    assert!(matches!(result, Err(ApplyError::SchemaMismatch { .. })));
    assert_eq!(store.last_seq(), 0);
    assert!(store.context(&ContextId::new("c-1")).is_none());
    assert_eq!(store.rejections().len(), 1);
}

#[test]
fn duplicate_and_older_seq_are_ignored() {
    let mut store = Store::new();
    let first =
        EventEnvelope::new(5, Timestamp(1), DomainEvent::ContextUpserted(context("c-1", false)));
    let changes = store.apply(&first).expect("first apply");
    assert!(changes.contexts.contains(&ContextId::new("c-1")));

    let mut replay = first.clone();
    replay.event = DomainEvent::ContextUpserted(context("c-1", true));
    let again = store.apply(&replay).expect("duplicate is not an error");
    assert!(again.is_empty());
    replay.seq = 3;
    assert!(store.apply(&replay).expect("older seq").is_empty());

    assert_eq!(store.last_seq(), 5);
    assert!(!store.context(&ContextId::new("c-1")).expect("context").pinned);
}

#[test]
fn illegal_lifecycle_transition_is_rejected_and_recorded() {
    let mut feed = seeded();
    feed.ok(30, set_lifecycle("m-1", Lifecycle::Done));

    let result = feed.push(40, set_lifecycle("m-1", Lifecycle::InProgress));

    assert!(matches!(
        result,
        Err(ApplyError::IllegalTransition { from: Lifecycle::Done, to: Lifecycle::InProgress, .. })
    ));
    let moment = feed.store.moment(&MomentId::new("m-1")).expect("moment");
    assert_eq!(moment.lifecycle, Lifecycle::Done);
    let last = feed.store.rejections().last().expect("rejection recorded");
    assert_eq!(last.seq, feed.seq);
    // The rejected seq is consumed: redelivery is a no-op, not a second error.
    let redelivered =
        EventEnvelope::new(feed.seq, Timestamp(40), set_lifecycle("m-1", Lifecycle::InProgress));
    assert!(feed.store.apply(&redelivered).expect("redelivery").is_empty());
}

#[test]
fn rejected_event_changes_nothing_even_with_valid_parts() {
    let mut feed = seeded();
    // Legal lifecycle but a payload of a different kind: whole event rejected.
    let result = feed.push(
        30,
        DomainEvent::MomentUpdated {
            moment_id: MomentId::new("m-1"),
            lifecycle: Some(Lifecycle::NeedsMe),
            kind: Some(surface("s-1")),
        },
    );
    assert_eq!(result, Err(ApplyError::KindMismatch(MomentId::new("m-1"))));
    let moment = feed.store.moment(&MomentId::new("m-1")).expect("moment");
    assert_eq!(moment.lifecycle, Lifecycle::Pending);
    assert!(feed.store.needs_me_since(&MomentId::new("m-1")).is_none());
}

#[test]
fn changes_list_exactly_the_touched_ids() {
    let mut feed = seeded();
    let changes = feed.ok(30, set_lifecycle("m-1", Lifecycle::InProgress));
    assert_eq!(changes.moments.len(), 1);
    assert!(changes.moments.contains(&MomentId::new("m-1")));
    assert!(changes.contexts.is_empty());
    assert!(!changes.pin_order);

    let changes = feed.ok(40, DomainEvent::ContextUpserted(context("c-2", true)));
    assert!(changes.contexts.contains(&ContextId::new("c-2")));
    assert!(changes.pin_order);

    let changes = feed.ok(50, DomainEvent::SourceStatusChanged(SourceStatus::Connected));
    assert!(changes.source_status);
    let unchanged = feed.ok(60, DomainEvent::SourceStatusChanged(SourceStatus::Connected));
    assert!(unchanged.is_empty());
}

#[test]
fn moment_requires_known_context_and_unique_id() {
    let mut feed = seeded();
    let unknown = feed.push(30, added(moment("m-2", "c-x", 30, message("x"), Lifecycle::Pending)));
    assert_eq!(unknown, Err(ApplyError::UnknownContext(ContextId::new("c-x"))));
    let dup = feed.push(31, added(moment("m-1", "c-1", 31, message("x"), Lifecycle::Pending)));
    assert_eq!(dup, Err(ApplyError::DuplicateMoment(MomentId::new("m-1"))));
}

#[test]
fn needs_me_since_tracks_envelope_time() {
    let mut feed = seeded();
    feed.ok(30, set_lifecycle("m-1", Lifecycle::NeedsMe));
    assert_eq!(feed.store.needs_me_since(&MomentId::new("m-1")), Some(Timestamp(30)));
    feed.ok(40, set_lifecycle("m-1", Lifecycle::InProgress));
    assert_eq!(feed.store.needs_me_since(&MomentId::new("m-1")), None);
}

#[test]
fn surface_patches_apply_in_order() {
    let mut feed = seeded();
    feed.ok(30, added(moment("m-s", "c-1", 30, surface("s-1"), Lifecycle::InProgress)));
    feed.ok(
        40,
        DomainEvent::SurfacePatched {
            moment_id: MomentId::new("m-s"),
            surface_id: SurfaceId::new("s-1"),
            ops: vec![
                SurfaceOp::AppendText { block_id: BlockId::new("b-text"), text: ", world".into() },
                SurfaceOp::AppendBlocks(vec![Block::new(
                    BlockId::new("b-progress"),
                    BlockKind::Progress,
                    json!({ "value": 1.0 }),
                )]),
                SurfaceOp::SetStreaming(false),
            ],
            base_hash: None,
        },
    );
    let moment = feed.store.moment(&MomentId::new("m-s")).expect("moment");
    let MomentKind::AgentSurface(payload) = &moment.kind else { panic!("not a surface") };
    assert_eq!(payload.blocks.len(), 2);
    assert_eq!(payload.blocks[0].data["text"], "Hello, world");
    assert!(!payload.streaming);
}

#[test]
fn failing_surface_patch_is_atomic() {
    let mut feed = seeded();
    feed.ok(30, added(moment("m-s", "c-1", 30, surface("s-1"), Lifecycle::InProgress)));
    let result = feed.push(
        40,
        DomainEvent::SurfacePatched {
            moment_id: MomentId::new("m-s"),
            surface_id: SurfaceId::new("s-1"),
            ops: vec![
                SurfaceOp::SetStreaming(false),
                SurfaceOp::AppendText { block_id: BlockId::new("missing"), text: "x".into() },
            ],
            base_hash: None,
        },
    );
    assert_eq!(result, Err(ApplyError::UnknownBlock(BlockId::new("missing"))));
    let moment = feed.store.moment(&MomentId::new("m-s")).expect("moment");
    let MomentKind::AgentSurface(payload) = &moment.kind else { panic!("not a surface") };
    assert!(payload.streaming, "first op must not have been applied");

    let wrong = feed.push(
        50,
        DomainEvent::SurfacePatched {
            moment_id: MomentId::new("m-s"),
            surface_id: SurfaceId::new("s-other"),
            ops: Vec::new(),
            base_hash: None,
        },
    );
    assert!(matches!(wrong, Err(ApplyError::SurfaceMismatch { .. })));
}

#[test]
fn grant_resolves_once() {
    let mut feed = seeded();
    feed.ok(30, DomainEvent::GrantRequested(grant("g-1", Risk::High)));
    let resolve = || DomainEvent::GrantResolved {
        grant_id: GrantId::new("g-1"),
        state: GrantState::Approved,
    };
    feed.ok(40, resolve());
    assert_eq!(feed.push(50, resolve()), Err(ApplyError::GrantNotPending(GrantId::new("g-1"))));
    let back_to_pending = feed.push(
        60,
        DomainEvent::GrantResolved { grant_id: GrantId::new("g-1"), state: GrantState::Pending },
    );
    assert_eq!(back_to_pending, Err(ApplyError::InvalidGrantResolution(GrantId::new("g-1"))));
}

#[test]
fn removing_a_context_drops_its_moments_and_pin() {
    let mut feed = Feed::new();
    feed.ok(1, DomainEvent::ContextUpserted(context("c-1", true)));
    feed.ok(2, DomainEvent::ContextUpserted(context("c-2", false)));
    feed.ok(3, added(moment("m-1", "c-1", 3, message("a"), Lifecycle::NeedsMe)));
    feed.ok(4, added(moment("m-2", "c-2", 4, message("b"), Lifecycle::Pending)));

    let changes = feed.ok(5, DomainEvent::ContextRemoved { context_id: ContextId::new("c-1") });

    assert!(changes.pin_order);
    assert!(changes.moments.contains(&MomentId::new("m-1")));
    assert!(feed.store.moment(&MomentId::new("m-1")).is_none());
    assert!(feed.store.moment(&MomentId::new("m-2")).is_some());
    assert!(feed.store.pin_order().is_empty());
}

#[test]
fn unknown_node_health_is_rejected() {
    let mut feed = Feed::new();
    let result = feed.push(
        1,
        DomainEvent::NodeHealthChanged { node_id: NodeId::new("n-x"), health: NodeHealth::Healthy },
    );
    assert_eq!(result, Err(ApplyError::UnknownNode(NodeId::new("n-x"))));
}

#[test]
fn rejection_log_is_bounded() {
    let mut feed = Feed::new();
    for i in 0..(MAX_REJECTIONS + 10) {
        let _ = feed.push(i as i64, set_lifecycle("missing", Lifecycle::Done));
    }
    assert_eq!(feed.store.rejections().len(), MAX_REJECTIONS);
    assert_eq!(feed.store.rejections()[0].seq, 11);
}

#[test]
fn replay_is_deterministic() {
    let build = || {
        let mut feed = seeded();
        feed.ok(30, DomainEvent::ContextUpserted(context("c-2", true)));
        feed.ok(40, set_lifecycle("m-1", Lifecycle::NeedsMe));
        feed.ok(50, DomainEvent::GrantRequested(grant("g-1", Risk::Low)));
        feed.store
    };
    assert_eq!(format!("{:?}", build()), format!("{:?}", build()));
}

#[test]
fn listings_are_recorded_once_and_tell_empty_from_loading() {
    let mut feed = Feed::new();
    assert!(!feed.store.is_listed(Listing::Nodes));
    let changes = feed.ok(1, DomainEvent::ListingsLoaded(vec![Listing::Nodes, Listing::Accounts]));
    assert!(changes.listings);
    assert!(feed.store.is_listed(Listing::Nodes) && feed.store.is_listed(Listing::Accounts));
    assert!(!feed.store.is_listed(Listing::Contacts));
    let again = feed.ok(2, DomainEvent::ListingsLoaded(vec![Listing::Nodes]));
    assert!(again.is_empty(), "a repeated listing changes nothing");
    let empty = feed.store.nodes().next().is_none();
    assert_eq!(ListState::of(feed.store.is_listed(Listing::Nodes), empty), ListState::Empty);
    assert_eq!(ListState::of(feed.store.is_listed(Listing::Contacts), true), ListState::Loading);
}

#[test]
fn failures_are_counted_past_the_cap_and_new_ones_found() {
    let mut feed = Feed::new();
    let fail = |n: usize| {
        DomainEvent::EffectFailed(EffectFailure {
            method: format!("m-{n}"),
            code: FailureCode::Transport,
            message: String::new(),
        })
    };
    for n in 0..MAX_FAILURES {
        feed.ok(1, fail(n));
    }
    let since = feed.store.failures_total();
    assert!(feed.store.failures_since(since).is_empty());
    feed.ok(2, fail(100));
    feed.ok(2, fail(101));
    assert_eq!(feed.store.failures_total(), since + 2);
    assert_eq!(feed.store.failures().len(), MAX_FAILURES, "the kept list stays capped");
    let new: Vec<&str> =
        feed.store.failures_since(since).iter().map(|f| f.method.as_str()).collect();
    assert_eq!(new, ["m-100", "m-101"]);
    assert_eq!(feed.store.failures_since(0).len(), MAX_FAILURES, "only kept failures");
}
