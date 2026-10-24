// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Agent surfaces through the daemon: a fake daemon that holds surfaces on
//! the protocol's own state machine (every hash is the protocol's snapshot
//! hash and stream state), publishes `surface.changed` metadata on the
//! event stream, and answers `surface.get`/`list`/`action`/`request_resync`
//! as the pinned daemon (8eebac1) does.
//! The AI participant of scenarios s10 and s11 is played as the author.
//!
//! Each test waits until the desktop shows one version before the author
//! sends the next, so the mapping of each version is deterministic; one
//! test sends several versions at once on purpose (a coalesced read).

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::*;
use ramflux_desktop_core::{
    AccountId, AgentSurfacePayload, ContextId, DomainEvent, Effect, EventEnvelope, Intent,
    IntentKind, MomentKind, Source as _, Store, SurfaceEvent, SurfaceId, SurfaceRecord,
    SurfaceState, SyncMode, Timestamp, UpdateRequest, handle_intent,
};
use ramflux_desktop_sdk::surface::author::Author;
use ramflux_desktop_test::fake_surfaces::{
    AUTHOR_TICK, FIRST_AUTHOR_TIME, FakeSurfaces, surface_ref,
};
use ramflux_desktop_test::load_fixture;
use serde_json::{Value, json};

const ACCOUNT: &str = "acct-main";
const GROUP: &str = "thread-team";
const BOT: &str = "principal-bo";

struct World {
    fake: FakeBus,
    surfaces: Arc<FakeSurfaces>,
    author: Author,
}

impl World {
    fn start(name: &str, message_list: Option<Value>) -> (Self, PathBuf) {
        let path = socket_path(&format!("sf-{name}"));
        let surfaces = Arc::new(FakeSurfaces::default());
        let held = surfaces.clone();
        let fake = FakeBus::start(&path, move |method, _, body| {
            if let Some(reply) = held.handle(method, body) {
                return reply;
            }
            match (method, &message_list) {
                ("message.list", Some(list)) => Reply::Ok(list.clone()),
                _ => fixture(method),
            }
        });
        fake.enable_stream();
        (Self { fake, surfaces, author: Author::default() }, path)
    }

    /// The author sends `event` in `conversation`; the daemon applies it
    /// and, when `publish`, announces it. Returns whether it was accepted.
    fn send(
        &mut self,
        conversation: &str,
        author: &str,
        event: &SurfaceEvent,
        publish: bool,
    ) -> bool {
        let surfaces = self.surfaces.clone();
        let mut reference = None;
        let accepted = self
            .author
            .send(conversation, event, |bytes| match surfaces.author_event(author, bytes) {
                Ok(r) => {
                    reference = Some(r);
                    true
                }
                Err(_) => false,
            })
            .expect("authored");
        let reference = reference
            .unwrap_or_else(|| surface_ref(conversation, author, event.surface_id().as_str()));
        if publish {
            self.announce(&reference);
        }
        accepted
    }

    fn announce(&self, reference: &str) {
        self.fake.publish(ACCOUNT, "surface.changed", self.surfaces.meta(reference));
    }
}

fn streaming(path: &std::path::Path) -> Fed {
    let hub = ramflux_desktop_sdk::SdkHub::start(config(path)).expect("bridge");
    let mut account = Fed::new(hub.account_source(AccountId::new(ACCOUNT)));
    account.until("the stream is open", |s, _| {
        s.context(&ContextId::new(GROUP)).is_some()
            && s.sync_status().is_some_and(|sync| sync.mode == SyncMode::Stream)
    });
    account
}

fn record(store: &Store, reference: &str) -> Option<SurfaceRecord> {
    let moment = store.surface_moment(&SurfaceId::new(reference))?;
    store.surface(moment).cloned()
}

fn payload(store: &Store, reference: &str) -> AgentSurfacePayload {
    let moment = store.surface_moment(&SurfaceId::new(reference)).expect("surface moment");
    match &store.moment(moment).expect("moment").kind {
        MomentKind::AgentSurface(p) => p.clone(),
        other => panic!("not a surface: {other:?}"),
    }
}

/// Waits until the desktop shows `reference` at `version` with the
/// daemon's hash.
fn shown(account: &mut Fed, world: &World, reference: &str, version: u64) {
    let (_, hash) = world.surfaces.current(reference).expect("held");
    account.until(&format!("version {version}"), |s, _| {
        record(s, reference).is_some_and(|r| r.version == version && r.hash.as_str() == hash)
    });
}

/// The surface events the desktop received, by kind, from `from` on (the
/// times reported after each change left out).
fn kinds(account: &Fed, from: usize) -> Vec<&'static str> {
    account.envelopes[from..]
        .iter()
        .filter_map(|e| match &e.event {
            DomainEvent::Surface(event) => Some(match event {
                SurfaceEvent::Create { .. } => "Create",
                SurfaceEvent::Patch { .. } => "Patch",
                SurfaceEvent::StreamAppend { .. } => "StreamAppend",
                SurfaceEvent::Close { .. } => "Close",
                SurfaceEvent::Snapshot(_) => "Snapshot",
                SurfaceEvent::ActionRefused { .. } => "ActionRefused",
                SurfaceEvent::UpdateRequested { .. } => "UpdateRequested",
                SurfaceEvent::Removed { .. } => "Removed",
                SurfaceEvent::UpdatedAt { .. } => return None,
            }),
            _ => None,
        })
        .collect()
}

/// The author's time `events` events after the first (milliseconds).
fn author_time(events: i64) -> Timestamp {
    Timestamp((FIRST_AUTHOR_TIME + events * AUTHOR_TICK) * 1000)
}

/// A scenario's surface events, in order (timeline, then responses).
fn scenario_surfaces(name: &str) -> Vec<SurfaceEvent> {
    let scenario = load_fixture(name).expect("fixture");
    scenario
        .timeline
        .iter()
        .chain(scenario.responses.iter().flat_map(|r| &r.emit))
        .filter_map(|(_, e)| match e {
            DomainEvent::Surface(event) => Some(event.clone()),
            _ => None,
        })
        .collect()
}

/// What the scenario shows when its events are applied directly (the mock
/// path): each surface's final content.
fn mock_payloads(name: &str, events: &[SurfaceEvent]) -> Vec<(String, AgentSurfacePayload)> {
    let scenario = load_fixture(name).expect("fixture");
    let mut store = Store::new();
    let mut seq = 0;
    let mut apply = |store: &mut Store, event: DomainEvent| {
        seq += 1;
        let _ = store.apply(&EventEnvelope::new(seq, Timestamp(0), event));
    };
    for (_, event) in &scenario.timeline {
        if !matches!(event, DomainEvent::Surface(_)) {
            apply(&mut store, event.clone());
        }
    }
    for event in events {
        apply(&mut store, DomainEvent::Surface(event.clone()));
    }
    let mut ids: Vec<String> = events.iter().map(|e| e.surface_id().to_string()).collect();
    ids.dedup();
    ids.sort();
    ids.dedup();
    ids.into_iter()
        .map(|id| {
            let moment = store.surface_moment(&SurfaceId::new(&id)).expect("mock surface").clone();
            let MomentKind::AgentSurface(p) = store.moment(&moment).expect("moment").kind.clone()
            else {
                panic!("not a surface")
            };
            (id, p)
        })
        .collect()
}

fn same_content(a: &AgentSurfacePayload, b: &AgentSurfacePayload) -> bool {
    a.blocks == b.blocks && a.actions == b.actions && a.fallback_text == b.fallback_text
}

#[test]
fn s10_streams_and_patches_through_the_daemon_with_the_protocol_hash() {
    let (mut world, path) = World::start("s10", None);
    let mut account = streaming(&path);
    let events = scenario_surfaces("s10_ai_participant");
    let mark = account.envelopes.len();
    let mut versions = Vec::new();
    let reply = surface_ref(GROUP, BOT, "s-reply");
    let mut writing = Vec::new();
    for event in &events {
        assert!(world.send(GROUP, BOT, event, true), "the daemon accepts {event:?}");
        let reference = surface_ref(GROUP, BOT, event.surface_id().as_str());
        shown(&mut account, &world, &reference, event.version());
        versions.push((reference, event.version()));
        writing.push(payload(&account.store, &reply).streaming);
    }
    assert_eq!(
        kinds(&account, mark),
        ["Snapshot", "StreamAppend", "StreamAppend", "StreamAppend", "Snapshot", "Patch"],
        "first sight is the whole state; then appends and a patch, version by version"
    );
    assert_eq!(
        writing,
        [true, true, true, false, false, false],
        "the reply is written until the daemon reports its stream final (v4), not after"
    );
    let r = record(&account.store, &reply).expect("reply");
    assert_eq!(r.hash.as_str(), world.surfaces.current(&reply).expect("held").1);
    let moment = account.store.surface_moment(&SurfaceId::new(&reply)).expect("m");
    assert_eq!(
        (account.store.moment(moment).expect("moment").at, r.updated_at),
        (author_time(0), Some(author_time(3))),
        "posted and last updated by the author's clock: created, then three appends"
    );
    let moment =
        account.store.moment(account.store.surface_moment(&SurfaceId::new(&reply)).expect("m"));
    assert_eq!(moment.expect("moment").origin.as_str(), BOT, "the author is the origin");
    for (id, expected) in mock_payloads("s10_ai_participant", &events) {
        let shown = payload(&account.store, &surface_ref(GROUP, BOT, &id));
        assert!(same_content(&shown, &expected), "{id}: {shown:?} vs {expected:?}");
    }
    world.fake.shutdown();
}

#[test]
fn s11_conflict_action_answer_and_close_through_the_daemon() {
    let (mut world, path) = World::start("s11", None);
    let mut account = streaming(&path);
    let events = scenario_surfaces("s11_surface_versions");
    let reference = surface_ref(GROUP, BOT, "s-pick");
    // Timeline: create v1, patch v2, a patch against v1 (refused by the
    // daemon: the desktop keeps v2), the resend against v2.
    let mut timeline = events.iter().take(4);
    for event in timeline.by_ref().take(2) {
        assert!(world.send(GROUP, BOT, event, true));
        shown(&mut account, &world, &reference, event.version());
    }
    let conflicting = timeline.next().expect("v3 against v1");
    assert!(!world.send(GROUP, BOT, conflicting, true), "a base conflict is refused");
    account.settle(300);
    assert_eq!(record(&account.store, &reference).expect("r").version, 2);
    let resent = timeline.next().expect("v3 against v2");
    let mark = account.envelopes.len();
    assert!(world.send(GROUP, BOT, resent, true));
    shown(&mut account, &world, &reference, 3);
    assert_eq!(kinds(&account, mark), ["Patch"]);

    // A stale action (shown on v2: stale_version; on v3 with v2's content:
    // stale_hash) is refused by the daemon: nothing done.
    let (v2_hash, v3) = (world.author.hash("s-pick", 2).expect("v2").to_owned(), 3);
    for version in [2, 3] {
        let mark = account.envelopes.len();
        account.source.submit(Effect::Call {
            method: "surface.action".into(),
            body: json!({ "surface_id": reference, "version": version, "surface_hash": v2_hash,
                          "action_id": "thu", "values": null }),
        });
        account.until("the refusal", |_, envelopes| {
            envelopes[mark..].iter().any(|e| {
                matches!(&e.event, DomainEvent::Surface(SurfaceEvent::ActionRefused { version: v, .. }) if *v == version)
            })
        });
    }
    assert!(world.surfaces.acted().is_empty());

    // The user picks Thursday on v3: the action carries v3 and its hash.
    let r = record(&account.store, &reference).expect("r");
    let moment = account.store.surface_moment(&SurfaceId::new(&reference)).expect("m").clone();
    let effects = handle_intent(
        &account.store,
        Intent::human(IntentKind::SurfaceAction {
            moment_id: moment,
            surface_id: SurfaceId::new(&reference),
            action_id: "thu".into(),
            values: Value::Null,
            surface_hash: r.hash.clone(),
            version: r.version,
        }),
        Timestamp(0),
    )
    .expect("policy");
    for effect in effects {
        account.source.submit(effect);
    }
    account.until("the action reaches the daemon", |_, _| !world.surfaces.acted().is_empty());
    let acted = world.surfaces.acted();
    assert_eq!(
        (acted[0].version, acted[0].action_id.as_str(), acted[0].author.as_str()),
        (v3, "thu", BOT)
    );

    // The author answers: result patch (v4), close (v5).
    let answer: Vec<&SurfaceEvent> = events
        .iter()
        .skip(4)
        .filter(|e| match e {
            SurfaceEvent::Patch { ops, .. } => format!("{ops:?}").contains("Thursday"),
            _ => true,
        })
        .take(2)
        .collect();
    let mark = account.envelopes.len();
    for event in &answer {
        assert!(world.send(GROUP, BOT, event, true));
        shown(&mut account, &world, &reference, event.version());
    }
    assert_eq!(kinds(&account, mark), ["Patch", "Close"]);
    assert_eq!(record(&account.store, &reference).expect("r").state, SurfaceState::Closed);

    // An action on the closed surface: the daemon's typed refusal, shown
    // as its text (no message parsing), nothing acted.
    let (v5, v5_hash) = world.surfaces.current(&reference).expect("held");
    let mark = account.envelopes.len();
    account.source.submit(Effect::Call {
        method: "surface.action".into(),
        body: json!({ "surface_id": reference, "version": v5, "surface_hash": v5_hash,
                      "action_id": "thu", "values": null }),
    });
    account.until("the closed refusal", |_, envelopes| {
        envelopes[mark..].iter().any(|e| {
            matches!(&e.event, DomainEvent::EffectFailed(f)
                if f.code == ramflux_desktop_core::FailureCode::Daemon("closed".into())
                && f.message == r#"backend.failure.surface_action_rejected {"reason":"backend.surface.closed"}"#)
        })
    });
    assert_eq!(world.surfaces.acted().len(), 1, "only the Thursday action");
    let mut played: Vec<SurfaceEvent> = events.iter().take(2).cloned().collect();
    played.push(resent.clone());
    played.extend(answer.into_iter().cloned());
    let expected = &mock_payloads("s11_surface_versions", &played)[0].1;
    assert!(same_content(&payload(&account.store, &reference), expected));
    world.fake.shutdown();
}

#[test]
fn an_out_of_sync_surface_asks_the_author_and_shows_the_answer() {
    let (mut world, path) = World::start("sync", None);
    let mut account = streaming(&path);
    let events = scenario_surfaces("s11_surface_versions");
    let reference = surface_ref(GROUP, BOT, "s-pick");
    assert!(world.send(GROUP, BOT, &events[0], true));
    shown(&mut account, &world, &reference, 1);
    world.surfaces.set_out_of_sync(&reference, true);
    world.announce(&reference);
    account.until("out of sync", |s, _| {
        record(s, &reference).is_some_and(|r| r.state == SurfaceState::OutOfSync)
    });
    let moment = account.store.surface_moment(&SurfaceId::new(&reference)).expect("m").clone();
    let ask = |store: &Store| {
        handle_intent(
            store,
            Intent::human(IntentKind::RequestSurfaceUpdate {
                moment_id: moment.clone(),
                surface_id: SurfaceId::new(&reference),
            }),
            Timestamp(0),
        )
        .expect("an update can be requested")
    };
    let update = |s: &Store| record(s, &reference).and_then(|r| r.update);
    // The request goes to the author (surface.request_resync): sent.
    for effect in ask(&account.store) {
        account.source.submit(effect);
    }
    account.until("sent", |s, _| update(s) == Some(UpdateRequest::Sent));
    assert_eq!(world.surfaces.resyncs(), std::slice::from_ref(&reference));
    // Asked again within the daemon's window: its rate_limited answer.
    for effect in ask(&account.store) {
        account.source.submit(effect);
    }
    account.until("rate limited", |s, _| {
        update(s) == Some(UpdateRequest::Refused("backend.surface.rate_limited".into()))
    });
    assert_eq!(world.surfaces.resyncs().len(), 1, "the daemon did not send it again");
    // The next window: sent again.
    world.surfaces.next_rate_window();
    for effect in ask(&account.store) {
        account.source.submit(effect);
    }
    account.until("sent again", |s, _| update(s) == Some(UpdateRequest::Sent));
    assert_eq!(world.surfaces.resyncs().len(), 2);
    world.surfaces.set_out_of_sync(&reference, false);
    assert!(world.send(GROUP, BOT, &events[1], true));
    shown(&mut account, &world, &reference, 2);
    let r = record(&account.store, &reference).expect("r");
    assert_eq!((&r.state, &r.update), (&SurfaceState::Open, &None));
    // An in-sync surface: the daemon's typed error frame (not_out_of_sync),
    // shown in the surface. (The policy does not offer it; the daemon's
    // answer is still read by its code.)
    account.source.submit(Effect::Call {
        method: "surface.request_resync".into(),
        body: json!({ "moment_id": moment, "surface_id": reference }),
    });
    account.until("not out of sync", |s, _| {
        update(s) == Some(UpdateRequest::Refused("backend.surface.not_out_of_sync".into()))
    });
    world.fake.shutdown();
}

#[test]
fn skipped_versions_arrive_as_a_snapshot_and_writing_ends_when_final() {
    let (mut world, path) = World::start("skip", None);
    let mut account = streaming(&path);
    let events = scenario_surfaces("s10_ai_participant");
    let reference = surface_ref(GROUP, BOT, "s-reply");
    assert!(world.send(GROUP, BOT, &events[0], true));
    shown(&mut account, &world, &reference, 1);
    // Two appends before the desktop reads: one read, versions skipped.
    assert!(world.send(GROUP, BOT, &events[1], false));
    assert!(world.send(GROUP, BOT, &events[2], true));
    let mark = account.envelopes.len();
    shown(&mut account, &world, &reference, 3);
    assert_eq!(kinds(&account, mark), ["Snapshot"]);
    assert!(payload(&account.store, &reference).streaming, "the block grew: being written");
    // A change that is not an append does not end the stream: the daemon
    // still reports it open.
    let fallback = SurfaceEvent::Patch {
        surface_id: SurfaceId::new("s-reply"),
        version: 4,
        base_hash: ramflux_desktop_core::SurfaceHash::new("v3"),
        ops: vec![ramflux_desktop_core::SurfacePatchOp::SetFallback {
            fallback_text: "Orbit: done.".into(),
        }],
        hash: None,
    };
    let mark = account.envelopes.len();
    assert!(world.send(GROUP, BOT, &fallback, true));
    shown(&mut account, &world, &reference, 4);
    assert_eq!(kinds(&account, mark), ["Patch"]);
    let p = payload(&account.store, &reference);
    assert!(p.streaming, "still open");
    assert_eq!(p.fallback_text, "Orbit: done.");
    // The author ends the stream with an empty final append: the mark ends.
    let end = SurfaceEvent::StreamAppend {
        surface_id: SurfaceId::new("s-reply"),
        version: 5,
        block_id: ramflux_desktop_core::BlockId::new("b-reply"),
        text_delta: String::new(),
        is_final: true,
        hash: None,
    };
    let mark = account.envelopes.len();
    assert!(world.send(GROUP, BOT, &end, true));
    shown(&mut account, &world, &reference, 5);
    assert_eq!(kinds(&account, mark), ["StreamAppend"]);
    assert!(!payload(&account.store, &reference).streaming, "final: not written any more");
    world.fake.shutdown();
}

#[test]
fn a_surface_the_daemon_removes_leaves_the_conversation() {
    let (mut world, path) = World::start("removed", None);
    let mut account = streaming(&path);
    let reference = surface_ref(GROUP, BOT, "s-reply");
    assert!(world.send(GROUP, BOT, &scenario_surfaces("s10_ai_participant")[0], true));
    shown(&mut account, &world, &reference, 1);
    let mark = account.envelopes.len();
    world.surfaces.remove(&reference);
    world.announce(&reference);
    account.until("removed", |s, _| s.surface_moment(&SurfaceId::new(&reference)).is_none());
    assert_eq!(kinds(&account, mark), ["Removed"]);
    // A removal of a surface never shown changes nothing.
    world.fake.publish(
        ACCOUNT,
        "surface.changed",
        json!({ "surface_id": "other", "surface_ref": "other", "removed": true }),
    );
    account.settle(300);
    assert_eq!(kinds(&account, mark), ["Removed"]);
    world.fake.shutdown();
}

#[test]
fn surface_messages_are_not_messages_and_date_an_undated_surface() {
    let carrier: Vec<Value> = b"ramflux.surface.v2\n{}".iter().map(|b| json!(b)).collect();
    let mut list = frames()["message.list"].clone();
    let rows = list["messages"].as_array_mut().expect("rows");
    rows.push(
        json!({ "conversation_id": "dm-ana", "message_id": "m-s1", "sender_id": "device-ana",
        "encrypted_body": carrier, "content_type": "surface", "surface_id": "s-ana",
        "body_utf8": "Surface author=\"principal-ana\" fallback_text=\"Pick\"",
        "deleted": false, "created_at": 1_790_000_100_i64, "receipts": [] }),
    );
    let (mut world, path) = World::start("rows", Some(list));
    // An old carrier without the author's time: the message list dates it.
    world.surfaces.undated();
    let mut account = streaming(&path);
    let create = scenario_surfaces("s11_surface_versions")[0].clone();
    let SurfaceEvent::Create { blocks, actions, fallback_text, .. } = create else { panic!() };
    let create = SurfaceEvent::Create {
        moment_id: "m".into(),
        context_id: ContextId::new("dm-ana"),
        origin: "principal-ana".into(),
        surface_id: SurfaceId::new("s-ana"),
        version: 1,
        blocks,
        actions,
        fallback_text,
        hash: None,
    };
    assert!(world.send("dm-ana", "principal-ana", &create, false));
    // Looking at the conversation reads its messages and its surfaces.
    account.source.focus(Some(&ContextId::new("dm-ana")));
    let reference = surface_ref("dm-ana", "principal-ana", "s-ana");
    shown(&mut account, &world, &reference, 1);
    let moment = account.store.surface_moment(&SurfaceId::new(&reference)).expect("m").clone();
    assert_eq!(account.store.moment(&moment).expect("moment").at, Timestamp(1_790_000_100_000));
    let messages = account
        .store
        .context_moments(&ContextId::new("dm-ana"))
        .filter(|m| matches!(m.kind, MomentKind::Message(_)))
        .count();
    assert_eq!(messages, 2, "the two text messages only; the surface message is the surface");
    world.fake.shutdown();
}
