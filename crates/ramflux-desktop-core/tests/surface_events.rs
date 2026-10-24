// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The versioned surface events: every transition and every reject path,
//! the renderer limits at the reducer edge, and how actions bind to the
//! version they were shown on.

mod common;

use common::*;
use ramflux_desktop_core::conformance::LimitKind;
use ramflux_desktop_core::*;
use serde_json::json;

const NOW: Timestamp = Timestamp(1_000);

fn sid() -> SurfaceId {
    SurfaceId::new("s-1")
}

fn mid() -> MomentId {
    MomentId::new("m-s")
}

fn seeded() -> Feed {
    let mut feed = Feed::new();
    feed.ok(
        1,
        DomainEvent::ParticipantUpserted(participant(
            "p-ai",
            ParticipantKind::Agent,
            "Orbit",
            false,
        )),
    );
    feed.ok(1, DomainEvent::ContextUpserted(context("c-1", false)));
    feed
}

fn action(id: &str, risk: Risk) -> SurfaceAction {
    SurfaceAction { id: id.to_owned(), label: id.to_owned(), risk, disabled_reason: None }
}

fn create(blocks: Vec<Block>, actions: Vec<SurfaceAction>) -> DomainEvent {
    DomainEvent::Surface(SurfaceEvent::Create {
        moment_id: mid(),
        context_id: ContextId::new("c-1"),
        origin: ParticipantId::new("p-ai"),
        surface_id: sid(),
        version: 1,
        blocks,
        actions,
        fallback_text: "Orbit replied".to_owned(),
        hash: None,
    })
}

fn reply() -> DomainEvent {
    create(
        vec![Block::new("b-reply", BlockKind::Text, json!({ "text": "" }))],
        vec![action("ok", Risk::Low)],
    )
}

fn append(version: u64, text: &str, is_final: bool) -> DomainEvent {
    DomainEvent::Surface(SurfaceEvent::StreamAppend {
        surface_id: sid(),
        version,
        block_id: BlockId::new("b-reply"),
        text_delta: text.to_owned(),
        is_final,
        hash: None,
    })
}

fn patch(version: u64, base: SurfaceHash, ops: Vec<SurfacePatchOp>) -> DomainEvent {
    DomainEvent::Surface(SurfaceEvent::Patch {
        surface_id: sid(),
        version,
        base_hash: base,
        ops,
        hash: None,
    })
}

fn close(version: u64) -> DomainEvent {
    DomainEvent::Surface(SurfaceEvent::Close { surface_id: sid(), version })
}

fn record(feed: &Feed) -> SurfaceRecord {
    feed.store.surface(&mid()).cloned().expect("surface record")
}

fn payload(feed: &Feed) -> AgentSurfacePayload {
    match &feed.store.moment(&mid()).expect("moment").kind {
        MomentKind::AgentSurface(p) => p.clone(),
        other => panic!("not a surface: {other:?}"),
    }
}

fn text(feed: &Feed) -> String {
    payload(feed).blocks[0].data["text"].as_str().unwrap_or_default().to_owned()
}

fn hash(feed: &Feed) -> SurfaceHash {
    record(feed).hash
}

fn act(feed: &Feed, hash: SurfaceHash, version: u64) -> Result<Vec<Effect>, IntentError> {
    handle_intent(
        &feed.store,
        Intent::human(IntentKind::SurfaceAction {
            moment_id: mid(),
            surface_id: sid(),
            action_id: "ok".to_owned(),
            values: json!({ "pick": "thu" }),
            surface_hash: hash,
            version,
        }),
        NOW,
    )
}

#[test]
fn create_makes_version_one_and_opens_its_empty_text_blocks() {
    let mut feed = seeded();
    let changes = feed.ok(10, reply());
    assert!(changes.moments.contains(&mid()));
    let moment = feed.store.moment(&mid()).expect("moment");
    assert_eq!((moment.at, moment.lifecycle), (Timestamp(10), Lifecycle::Done));
    assert_eq!(moment.origin.as_str(), "p-ai");
    let r = record(&feed);
    assert_eq!((r.version, &r.state), (1, &SurfaceState::Open));
    assert_eq!(r.hash, SurfaceHash::of_version(1), "no source hash: the version label");
    assert!(payload(&feed).streaming, "an empty text block is a stream about to start");
    assert_eq!(feed.store.surface_moment(&sid()), Some(&mid()));
}

#[test]
fn create_rejects_bad_versions_duplicates_and_missing_fallbacks() {
    let mut feed = seeded();
    let mut wrong = reply();
    if let DomainEvent::Surface(SurfaceEvent::Create { version, .. }) = &mut wrong {
        *version = 2;
    }
    assert_eq!(
        feed.push(10, wrong),
        Err(ApplyError::SurfaceVersion { surface: sid(), expected: 1, found: 2 })
    );
    let mut blank = reply();
    if let DomainEvent::Surface(SurfaceEvent::Create { fallback_text, .. }) = &mut blank {
        *fallback_text = "  ".to_owned();
    }
    assert_eq!(feed.push(11, blank), Err(ApplyError::MissingFallback(sid())));
    let twice = create(
        vec![
            Block::new("b", BlockKind::Text, json!({ "text": "a" })),
            Block::new("b", BlockKind::Text, json!({ "text": "b" })),
        ],
        Vec::new(),
    );
    assert_eq!(feed.push(12, twice), Err(ApplyError::DuplicateBlock(BlockId::new("b"))));
    let mut elsewhere = reply();
    if let DomainEvent::Surface(SurfaceEvent::Create { context_id, .. }) = &mut elsewhere {
        *context_id = ContextId::new("c-none");
    }
    assert_eq!(feed.push(13, elsewhere), Err(ApplyError::UnknownContext(ContextId::new("c-none"))));
    assert!(feed.store.surface(&mid()).is_none(), "nothing was stored");

    feed.ok(14, reply());
    let mut again = reply();
    if let DomainEvent::Surface(SurfaceEvent::Create { moment_id, .. }) = &mut again {
        *moment_id = MomentId::new("m-other");
    }
    assert_eq!(feed.push(15, again), Err(ApplyError::DuplicateSurface(sid())));
    assert_eq!(
        feed.push(16, append(9, "x", false)).map(|_| ()),
        Err(ApplyError::SurfaceVersion { surface: sid(), expected: 2, found: 9 })
    );
    let unknown = DomainEvent::Surface(SurfaceEvent::Close {
        surface_id: SurfaceId::new("s-none"),
        version: 2,
    });
    assert_eq!(feed.push(17, unknown), Err(ApplyError::UnknownSurface(SurfaceId::new("s-none"))));
}

#[test]
fn streaming_appends_in_place_until_final() {
    let mut feed = seeded();
    feed.ok(10, reply());
    feed.ok(11, append(2, "Two items ", false));
    feed.ok(12, append(3, "are open.", false));
    assert_eq!(text(&feed), "Two items are open.");
    let r = record(&feed);
    assert_eq!(r.version, 3);
    assert_eq!(r.streams[&BlockId::new("b-reply")], StreamState { chars: 19, finished: false });
    assert!(payload(&feed).streaming);
    feed.ok(13, append(4, "", true));
    assert!(!payload(&feed).streaming, "the last stream ended");
    assert!(record(&feed).streams[&BlockId::new("b-reply")].finished);
    assert_eq!(
        feed.push(14, append(5, "more", false)),
        Err(ApplyError::StreamFinished(BlockId::new("b-reply")))
    );
    assert_eq!(text(&feed), "Two items are open.", "a refused append changes nothing");
}

#[test]
fn streaming_into_a_block_without_text_is_refused() {
    let mut feed = seeded();
    feed.ok(
        10,
        create(
            vec![
                Block::new("b-reply", BlockKind::Progress, json!({ "value": 0.5 })),
                Block::new("b-list", BlockKind::List, json!({ "items": ["a"] })),
            ],
            Vec::new(),
        ),
    );
    assert_eq!(
        feed.push(11, append(2, "x", false)),
        Err(ApplyError::NotTextBlock(BlockId::new("b-reply")))
    );
    let missing = DomainEvent::Surface(SurfaceEvent::StreamAppend {
        surface_id: sid(),
        version: 2,
        block_id: BlockId::new("b-none"),
        text_delta: "x".to_owned(),
        is_final: false,
        hash: None,
    });
    assert_eq!(feed.push(12, missing), Err(ApplyError::UnknownBlock(BlockId::new("b-none"))));
    assert_eq!(record(&feed).version, 1);
}

#[test]
fn a_delta_over_four_kilobytes_leaves_only_the_fallback() {
    let mut feed = seeded();
    feed.ok(10, reply());
    feed.ok(11, append(2, "Short start. ", false));
    let changes = feed.ok(12, append(3, &"x".repeat(4_097), false));
    assert!(changes.moments.contains(&mid()));
    let r = record(&feed);
    assert_eq!(r.state, SurfaceState::OverLimit(LimitKind::TextDelta));
    assert_eq!(r.version, 2, "the refused append is not a version");
    let p = payload(&feed);
    assert!(p.blocks.is_empty() && p.actions.is_empty(), "never a partial rendering");
    assert_eq!(p.fallback_text, "Orbit replied");
    assert_eq!(
        feed.push(13, append(3, "y", false)),
        Err(ApplyError::SurfaceOverLimit(sid())),
        "an over-limit surface takes no more changes"
    );
    assert!(matches!(act(&feed, r.hash.clone(), r.version), Err(IntentError::SurfaceOverLimit(_))));
    feed.ok(14, close(3));
    assert_eq!(record(&feed).state, SurfaceState::Closed);
}

#[test]
fn a_stream_past_the_state_bound_leaves_only_the_fallback() {
    // The protocol bounds the whole state at 16,384 bytes; a stream reaches
    // that bound before its own character bound.
    let mut feed = seeded();
    feed.ok(10, reply());
    let chunk = "y".repeat(4_096);
    for version in 2..=4 {
        feed.ok(10 + i64::try_from(version).unwrap_or(0), append(version, &chunk, false));
    }
    assert_eq!(record(&feed).streams[&BlockId::new("b-reply")].chars, 12_288);
    feed.ok(20, append(5, &chunk, false));
    assert_eq!(record(&feed).state, SurfaceState::OverLimit(LimitKind::DataBytes));
    assert!(payload(&feed).blocks.is_empty());
}

#[test]
fn a_patch_against_the_current_version_applies_every_op() {
    let mut feed = seeded();
    feed.ok(10, reply());
    let base = hash(&feed);
    let ops = vec![
        SurfacePatchOp::InsertBlock {
            index: 0,
            block: Block::new("b-head", BlockKind::Heading, json!({ "text": "Checklist" })),
        },
        SurfacePatchOp::ReplaceBlock {
            block: Block::new("b-reply", BlockKind::Text, json!({ "text": "Done" })),
        },
        SurfacePatchOp::InsertBlock {
            index: 2,
            block: Block::new("b-tail", BlockKind::Text, json!({ "text": "tail" })),
        },
        SurfacePatchOp::RemoveBlock { block_id: BlockId::new("b-tail") },
        SurfacePatchOp::SetActions {
            actions: vec![action("ok", Risk::Low), action("more", Risk::High)],
        },
    ];
    feed.ok(11, patch(2, base.clone(), ops));
    let p = payload(&feed);
    let ids: Vec<&str> = p.blocks.iter().map(|b| b.id.as_str()).collect();
    assert_eq!(ids, ["b-head", "b-reply"]);
    assert_eq!(p.actions.len(), 2);
    let r = record(&feed);
    assert_eq!((r.version, &r.state), (2, &SurfaceState::Open));
    assert_ne!(r.hash, base);
    assert_eq!(r.hash, SurfaceHash::of_version(2));
}

#[test]
fn a_patch_with_bad_ops_changes_nothing() {
    let mut feed = seeded();
    feed.ok(10, reply());
    let base = hash(&feed);
    let cases = [
        (
            SurfacePatchOp::ReplaceBlock {
                block: Block::new("b-none", BlockKind::Text, json!({ "text": "x" })),
            },
            ApplyError::UnknownBlock(BlockId::new("b-none")),
        ),
        (
            SurfacePatchOp::InsertBlock {
                index: 5,
                block: Block::new("b-new", BlockKind::Text, json!({ "text": "x" })),
            },
            ApplyError::InsertOutOfRange { block: BlockId::new("b-new"), index: 5 },
        ),
        (
            SurfacePatchOp::InsertBlock {
                index: 0,
                block: Block::new("b-reply", BlockKind::Text, json!({ "text": "x" })),
            },
            ApplyError::DuplicateBlock(BlockId::new("b-reply")),
        ),
        (
            SurfacePatchOp::RemoveBlock { block_id: BlockId::new("b-none") },
            ApplyError::UnknownBlock(BlockId::new("b-none")),
        ),
    ];
    for (op, error) in cases {
        assert_eq!(feed.push(11, patch(2, base.clone(), vec![op])), Err(error));
    }
    assert_eq!(
        feed.push(12, patch(3, base.clone(), Vec::new())),
        Err(ApplyError::SurfaceVersion { surface: sid(), expected: 2, found: 3 })
    );
    assert_eq!((record(&feed).version, hash(&feed)), (1, base));
}

#[test]
fn a_patch_on_another_base_is_a_conflict_until_the_author_resends() {
    let mut feed = seeded();
    feed.ok(10, reply());
    let first = hash(&feed);
    feed.ok(11, append(2, "Hello", true));
    let current = hash(&feed);
    // The author patched the version it saw before the stream.
    let stale = vec![SurfacePatchOp::SetActions { actions: vec![action("ok", Risk::Critical)] }];
    let changes = feed.ok(12, patch(3, first, stale));
    assert!(changes.moments.contains(&mid()), "the user sees the conflict");
    assert!(feed.store.surface_conflict(&mid()));
    assert!(matches!(record(&feed).state, SurfaceState::Conflict { .. }));
    assert_eq!((record(&feed).version, hash(&feed)), (2, current.clone()), "not applied");
    assert!(matches!(act(&feed, current.clone(), 2), Err(IntentError::SurfaceConflict(_))));
    // Resent against the current version: applied, conflict over.
    let fresh = vec![SurfacePatchOp::SetActions { actions: vec![action("ok", Risk::Low)] }];
    feed.ok(13, patch(3, current, fresh));
    assert!(!feed.store.surface_conflict(&mid()));
    assert_eq!(record(&feed).version, 3);
}

#[test]
fn a_patch_over_a_limit_is_not_rendered_partially() {
    let mut feed = seeded();
    feed.ok(10, reply());
    let many = (0..17).map(|i| action(&format!("a{i}"), Risk::Low)).collect();
    feed.ok(11, patch(2, hash(&feed), vec![SurfacePatchOp::SetActions { actions: many }]));
    let r = record(&feed);
    assert_eq!(r.state, SurfaceState::OverLimit(LimitKind::Actions));
    assert_eq!(r.version, 1);
    assert!(payload(&feed).actions.is_empty());
}

#[test]
fn a_surface_created_over_a_limit_keeps_only_its_fallback() {
    let mut feed = seeded();
    let blocks = (0..65)
        .map(|i| Block::new(format!("b{i}"), BlockKind::Text, json!({ "text": "x" })))
        .collect();
    feed.ok(10, create(blocks, vec![action("ok", Risk::Low)]));
    let r = record(&feed);
    assert_eq!((r.version, &r.state), (1, &SurfaceState::OverLimit(LimitKind::Blocks)));
    let p = payload(&feed);
    assert!(p.blocks.is_empty() && p.actions.is_empty());
    assert!(!p.streaming);
    assert!(matches!(act(&feed, r.hash, 1), Err(IntentError::SurfaceOverLimit(_))));
}

#[test]
fn close_freezes_the_surface() {
    let mut feed = seeded();
    feed.ok(10, reply());
    feed.ok(11, append(2, "Hi", false));
    let before = hash(&feed);
    feed.ok(12, close(3));
    let r = record(&feed);
    assert_eq!((r.version, &r.state), (3, &SurfaceState::Closed));
    assert_eq!(r.hash, before, "closing does not change the content");
    assert!(!payload(&feed).streaming);
    assert!(r.streams.values().all(|s| s.finished));
    assert_eq!(feed.push(13, append(4, "x", false)), Err(ApplyError::SurfaceClosed(sid())));
    assert_eq!(feed.push(14, close(4)), Err(ApplyError::SurfaceClosed(sid())));
    assert_eq!(
        feed.push(15, patch(4, before.clone(), Vec::new())),
        Err(ApplyError::SurfaceClosed(sid()))
    );
    assert!(matches!(act(&feed, before, 3), Err(IntentError::SurfaceClosed(_))));
    assert_eq!(feed.push(16, close(9)), Err(ApplyError::SurfaceClosed(sid())), "closed first");
}

#[test]
fn close_needs_the_next_version() {
    let mut feed = seeded();
    feed.ok(10, reply());
    assert_eq!(
        feed.push(11, close(3)),
        Err(ApplyError::SurfaceVersion { surface: sid(), expected: 2, found: 3 })
    );
}

#[test]
fn an_action_is_bound_to_the_version_it_was_shown_on() {
    let mut feed = seeded();
    feed.ok(10, reply());
    let shown = (hash(&feed), record(&feed).version);
    let effects = act(&feed, shown.0.clone(), shown.1).expect("current version");
    let body = effects[0].body().expect("call");
    assert_eq!(body["version"], 1);
    assert_eq!(body["surface_hash"], json!(shown.0));
    assert_eq!(body["values"], json!({ "pick": "thu" }), "the source hashes the arguments");
    feed.ok(11, append(2, "changed", false));
    assert!(
        matches!(act(&feed, shown.0.clone(), shown.1), Err(IntentError::SurfaceChanged(_))),
        "a stale action is never sent"
    );
    let now = hash(&feed);
    assert!(
        matches!(act(&feed, now.clone(), 1), Err(IntentError::SurfaceChanged(_))),
        "version too"
    );
    assert!(act(&feed, now, 2).is_ok());
}
