// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Surface versions: the content hash advances with every patch, a patch on
//! another base marks the surface in conflict instead of applying, and
//! actions are bound to the version they were shown on. Grant failures
//! close a grant for good.

mod common;

use common::*;
use ramflux_desktop_core::*;
use serde_json::json;

const NOW: Timestamp = Timestamp(100);

fn seeded() -> Feed {
    let mut feed = Feed::new();
    feed.ok(1, DomainEvent::ContextUpserted(context("c-1", false)));
    feed.ok(2, added(moment("m-s", "c-1", 2, surface("s-1"), Lifecycle::Done)));
    feed
}

fn hash(feed: &Feed) -> SurfaceHash {
    feed.store.surface_hash(&MomentId::new("m-s")).cloned().expect("surface hash")
}

fn patch(ops: Vec<SurfaceOp>, base: Option<SurfaceHash>) -> DomainEvent {
    DomainEvent::SurfacePatched {
        moment_id: MomentId::new("m-s"),
        surface_id: SurfaceId::new("s-1"),
        ops,
        base_hash: base,
    }
}

fn append(text: &str) -> SurfaceOp {
    SurfaceOp::AppendText { block_id: BlockId::new("b-text"), text: text.to_owned() }
}

fn run(shown: SurfaceHash) -> Intent {
    Intent::human(IntentKind::SurfaceAction {
        moment_id: MomentId::new("m-s"),
        surface_id: SurfaceId::new("s-1"),
        action_id: "run".to_owned(),
        values: serde_json::Value::Null,
        surface_hash: shown,
        version: 0,
    })
}

fn text_of(feed: &Feed) -> String {
    let MomentKind::AgentSurface(s) = &feed.store.moment(&MomentId::new("m-s")).expect("m").kind
    else {
        panic!("not a surface")
    };
    s.blocks[0].data["text"].as_str().unwrap_or_default().to_owned()
}

#[test]
fn an_unhashed_surface_is_labelled_by_its_version() {
    let feed = seeded();
    assert_eq!(hash(&feed), SurfaceHash::of_version(1));
}

#[test]
fn a_valid_patch_advances_the_hash_and_a_based_one_needs_the_current_base() {
    let mut feed = seeded();
    let first = hash(&feed);
    feed.ok(3, patch(vec![append(", world")], Some(first.clone())));
    let second = hash(&feed);
    assert_ne!(first, second, "a content patch advances the version");
    assert!(!feed.store.surface_conflict(&MomentId::new("m-s")));

    // A streaming flag is not content.
    feed.ok(4, patch(vec![SurfaceOp::SetStreaming(false)], Some(second.clone())));
    assert_eq!(hash(&feed), second);

    // A patch made against the first version is stale: not applied, conflict.
    let changes = feed.ok(5, patch(vec![append("!!")], Some(first)));
    assert!(changes.moments.contains(&MomentId::new("m-s")));
    assert_eq!(text_of(&feed), "Hello, world", "a stale patch is never applied");
    assert_eq!(hash(&feed), second);
    assert!(feed.store.surface_conflict(&MomentId::new("m-s")));
    assert!(feed.store.rejections().is_empty(), "a conflict is recorded, not rejected");
    assert_eq!(
        handle_intent(&feed.store, run(second), NOW).err(),
        Some(IntentError::SurfaceConflict(MomentId::new("m-s")))
    );

    // A full copy resolves the conflict and becomes the new version.
    feed.ok(
        6,
        DomainEvent::MomentUpdated {
            moment_id: MomentId::new("m-s"),
            lifecycle: None,
            kind: Some(surface("s-1")),
        },
    );
    assert!(!feed.store.surface_conflict(&MomentId::new("m-s")));
    let fresh = hash(&feed);
    assert!(handle_intent(&feed.store, run(fresh), NOW).is_ok());
}

#[test]
fn an_action_shown_on_an_older_version_is_refused() {
    let mut feed = seeded();
    let shown = hash(&feed);
    assert!(handle_intent(&feed.store, run(shown.clone()), NOW).is_ok());
    feed.ok(3, patch(vec![append(" again")], None));
    assert_eq!(
        handle_intent(&feed.store, run(shown), NOW).err(),
        Some(IntentError::SurfaceChanged(MomentId::new("m-s")))
    );
    let body = handle_intent(&feed.store, run(hash(&feed)), NOW).expect("current version");
    assert_eq!(body[0].body().expect("call")["surface_hash"], json!(hash(&feed)));
}

#[test]
fn a_confirmed_action_is_refused_when_the_surface_changed_during_the_hold() {
    let mut feed = seeded();
    // Make the action high risk so it waits for a hold.
    feed.ok(
        3,
        patch(
            vec![SurfaceOp::SetActions(vec![SurfaceAction {
                id: "run".to_owned(),
                label: "Run".to_owned(),
                risk: Risk::High,
                disabled_reason: None,
            }])],
            None,
        ),
    );
    let shown = hash(&feed);
    let effects = handle_intent(&feed.store, run(shown.clone()), NOW).expect("needs a hold");
    let Some(Effect::RequireConfirmation { intent }) = effects.into_iter().next() else {
        panic!("a high-risk action waits for the hold");
    };
    // The patch lands while the strip is open.
    feed.ok(4, patch(vec![append(" (changed)")], None));
    let confirmed = intent.confirmed_by_human(
        ConfirmationIssuer::new().issue_hold(MIN_HOLD_MS + 10).expect("a full hold"),
    );
    assert_eq!(
        handle_intent(&feed.store, confirmed, NOW).err(),
        Some(IntentError::SurfaceChanged(MomentId::new("m-s"))),
        "a hold never confirms content the user did not review"
    );
}

#[test]
fn actions_on_a_surface_over_a_limit_are_refused() {
    let mut feed = seeded();
    let many: Vec<Block> = (0..=conformance::Limits::DESKTOP.max_blocks)
        .map(|i| Block::new(format!("b-{i}"), BlockKind::Text, json!({ "text": "x" })))
        .collect();
    feed.ok(3, patch(vec![SurfaceOp::AppendBlocks(many)], None));
    assert_eq!(
        handle_intent(&feed.store, run(hash(&feed)), NOW).err(),
        Some(IntentError::SurfaceOverLimit(MomentId::new("m-s")))
    );
}

#[test]
fn a_failed_grant_is_closed_for_approve_and_deny() {
    let mut feed = seeded();
    feed.ok(3, DomainEvent::GrantRequested(grant("g-1", Risk::High)));
    let failed = |failure| DomainEvent::GrantFailed { grant_id: GrantId::new("g-1"), failure };
    feed.ok(4, failed(GrantFailure::SignedApprovalRequired));
    // The first failure is kept.
    feed.ok(5, failed(GrantFailure::Replay));
    let grant = feed.store.grant(&GrantId::new("g-1")).expect("grant");
    assert_eq!(grant.failure, Some(GrantFailure::SignedApprovalRequired));
    assert_eq!(grant.state, GrantState::Pending, "the decision state is untouched");
    for kind in [
        IntentKind::Approve { grant_id: GrantId::new("g-1") },
        IntentKind::Deny { grant_id: GrantId::new("g-1") },
    ] {
        assert_eq!(
            handle_intent(&feed.store, Intent::human(kind), NOW).err(),
            Some(IntentError::GrantFailed {
                grant: GrantId::new("g-1"),
                failure: GrantFailure::SignedApprovalRequired,
            })
        );
    }
    assert!(feed.push(6, failed(GrantFailure::Expired)).is_ok());
    let unknown = DomainEvent::GrantFailed {
        grant_id: GrantId::new("g-none"),
        failure: GrantFailure::Expired,
    };
    assert_eq!(feed.push(7, unknown), Err(ApplyError::UnknownGrant(GrantId::new("g-none"))));
}

#[test]
fn redacted_blocks_and_revealed_text_never_print() {
    let mut feed = seeded();
    let secret_block = Block {
        redacted: true,
        ..Block::new("b-secret", BlockKind::Text, json!({ "text": "4111 1111 1111 1111" }))
    };
    feed.ok(3, patch(vec![SurfaceOp::AppendBlocks(vec![secret_block.clone()])], None));
    feed.ok(
        4,
        added(moment(
            "m-p",
            "c-1",
            4,
            MomentKind::PrivacyReveal(PrivacyRevealPayload {
                label: "Card number".to_owned(),
                revealed: Some(RevealRecord { by: ParticipantId::new("p-ada"), at: Timestamp(4) }),
                content: Some(Protected::new("5500 0000 0000 0004")),
            }),
            Lifecycle::Done,
        )),
    );
    let printed = format!("{:?}", feed.store);
    assert!(!printed.contains("4111"), "redacted block data reached Debug");
    assert!(!printed.contains("5500"), "revealed text reached Debug");
    assert!(format!("{secret_block:?}").contains("[redacted]"));
    let MomentKind::PrivacyReveal(p) = &feed.store.moment(&MomentId::new("m-p")).expect("m").kind
    else {
        panic!("not a reveal")
    };
    assert_eq!(p.content.as_ref().map(Protected::expose), Some("5500 0000 0000 0004"));
    assert!(feed.store.body(&MomentId::new("m-p")).is_none(), "no body cache entry");
    assert!(feed.store.bodies().is_empty());
}
