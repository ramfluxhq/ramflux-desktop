// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! An authority's surface snapshots: creating a surface at any version,
//! skipping versions, out-of-sync state and what it refuses, the
//! authority's answers to actions and update requests, source hashes kept
//! as reported, the authority's last-updated time, removal, and the
//! authority's stream finality on a catch-up.

mod common;

use common::*;
use ramflux_desktop_core::*;
use serde_json::json;

const NOW: Timestamp = Timestamp(1_000);

fn sid() -> SurfaceId {
    SurfaceId::new("ref-1")
}

fn mid() -> MomentId {
    MomentId::new("m-ref-1")
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

fn snapshot(version: u64, hash: &str) -> SurfaceSnapshot {
    SurfaceSnapshot {
        moment_id: mid(),
        context_id: ContextId::new("c-1"),
        origin: ParticipantId::new("p-ai"),
        at: Some(Timestamp(500)),
        surface_id: sid(),
        version,
        hash: SurfaceHash::new(hash),
        actions: vec![SurfaceAction {
            id: "thu".to_owned(),
            label: "Thursday".to_owned(),
            risk: Risk::Low,
            disabled_reason: None,
        }],
        fallback_text: "Orbit asks which day".to_owned(),
        closed: false,
        out_of_sync: false,
        order: None,
    }
}

fn snap(s: SurfaceSnapshot) -> DomainEvent {
    DomainEvent::Surface(SurfaceEvent::Snapshot(Box::new(s)))
}

fn record(feed: &Feed) -> SurfaceRecord {
    feed.store.surface(&mid()).cloned().expect("record")
}

fn payload(feed: &Feed) -> AgentSurfacePayload {
    match &feed.store.moment(&mid()).expect("moment").kind {
        MomentKind::AgentSurface(p) => p.clone(),
        other => panic!("not a surface: {other:?}"),
    }
}

fn act(feed: &Feed) -> Result<Vec<Effect>, IntentError> {
    let r = record(feed);
    handle_intent(
        &feed.store,
        Intent::human(IntentKind::SurfaceAction {
            moment_id: mid(),
            surface_id: sid(),
            action_id: "thu".to_owned(),
            values: json!({}),
            surface_hash: r.hash,
            version: r.version,
        }),
        NOW,
    )
}

fn request_update(feed: &Feed) -> Result<Vec<Effect>, IntentError> {
    handle_intent(
        &feed.store,
        Intent::human(IntentKind::RequestSurfaceUpdate { moment_id: mid(), surface_id: sid() }),
        NOW,
    )
}

#[test]
fn an_out_of_sync_surface_refuses_actions_and_takes_an_update_request() {
    let mut feed = seeded();
    feed.ok(10, snap(snapshot(3, "Qh3")));
    assert!(matches!(request_update(&feed), Err(IntentError::SurfaceInSync(_))));
    let mut behind = snapshot(3, "Qh3");
    behind.out_of_sync = true;
    feed.ok(11, snap(behind));
    assert!(feed.store.surface_out_of_sync(&mid()));
    assert!(matches!(act(&feed), Err(IntentError::SurfaceOutOfSync(_))));
    let effects = request_update(&feed).expect("an update can be requested");
    let body = effects[0].body().expect("call");
    assert_eq!(effects[0].method(), Some(methods::SURFACE_REQUEST_RESYNC));
    assert_eq!(body["surface_id"], "ref-1");
    let agent = handle_intent(
        &feed.store,
        Intent::agent(
            ParticipantId::new("p-ai"),
            IntentKind::RequestSurfaceUpdate { moment_id: mid(), surface_id: sid() },
        ),
        NOW,
    );
    assert!(matches!(agent, Err(IntentError::AgentForbidden(_))));

    let answered = DomainEvent::Surface(SurfaceEvent::UpdateRequested {
        surface_id: sid(),
        refused: Some("backend.surface.rate_limited".into()),
    });
    feed.ok(12, answered);
    assert!(matches!(record(&feed).update, Some(UpdateRequest::Refused(_))));
    let sent =
        DomainEvent::Surface(SurfaceEvent::UpdateRequested { surface_id: sid(), refused: None });
    feed.ok(13, sent);
    assert_eq!(record(&feed).update, Some(UpdateRequest::Sent));
    // The author's current state arrives: in sync, the request is done.
    feed.ok(14, snap(snapshot(4, "Qh4")));
    let r = record(&feed);
    assert_eq!((&r.state, &r.update), (&SurfaceState::Open, &None));
    assert!(act(&feed).is_ok());
}

#[test]
fn a_refused_action_asks_the_user_to_review_the_surface_again() {
    let mut feed = seeded();
    feed.ok(10, snap(snapshot(3, "Qh3")));
    let refused =
        DomainEvent::Surface(SurfaceEvent::ActionRefused { surface_id: sid(), version: 3 });
    let changes = feed.ok(11, refused);
    assert!(changes.surface_refusals.contains(&mid()));
    assert_eq!(record(&feed).version, 3, "nothing else changes");
    let unknown = DomainEvent::Surface(SurfaceEvent::ActionRefused {
        surface_id: SurfaceId::new("x"),
        version: 1,
    });
    assert_eq!(feed.push(12, unknown), Err(ApplyError::UnknownSurface(SurfaceId::new("x"))));
}

#[test]
fn the_plain_text_timeline_says_when_a_surface_is_out_of_sync() {
    let mut feed = seeded();
    let mut behind = snapshot(2, "Qh2");
    behind.out_of_sync = true;
    feed.ok(10, snap(behind));
    let text = export::timeline_text(&feed.store, &ContextId::new("c-1")).join("\n");
    assert!(text.contains("Orbit asks which day [out of sync]"), "{text}");
}

#[test]
fn a_snapshot_creates_the_surface_at_its_version_with_the_reported_hash() {
    let mut feed = seeded();
    let changes = feed.ok(10, snap(snapshot(5, "Qh5")));
    assert!(changes.moments.contains(&mid()));
    let moment = feed.store.moment(&mid()).expect("moment");
    assert_eq!(moment.at, Timestamp(500), "posted time from the source");
    let r = record(&feed);
    assert_eq!((r.version, r.hash.as_str(), &r.state), (5, "Qh5", &SurfaceState::Open));
    assert!(act(&feed).is_ok());
}

#[test]
fn a_snapshot_may_skip_versions_but_never_goes_back_or_reopens_a_closed_surface() {
    let mut feed = seeded();
    feed.ok(10, snap(snapshot(2, "Qh2")));
    feed.ok(11, snap(snapshot(9, "Qh9")));
    assert_eq!(record(&feed).version, 9);
    assert_eq!(payload(&feed).fallback_text, "Orbit asks which day");
    assert_eq!(
        feed.push(12, snap(snapshot(8, "Qh8"))),
        Err(ApplyError::SurfaceVersion { surface: sid(), expected: 9, found: 8 })
    );
    let mut closed = snapshot(10, "Qh9");
    closed.closed = true;
    feed.ok(13, snap(closed));
    assert_eq!(record(&feed).state, SurfaceState::Closed);
    assert_eq!(feed.push(14, snap(snapshot(11, "Qh11"))), Err(ApplyError::SurfaceClosed(sid())));
}
