// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Every bundled scenario keeps the store's indexes equal to a full scan at
//! every step, and the mock source delivers evicted message bodies again.

use ramflux_desktop_core::{
    ContextId, Lifecycle, MomentId, MomentKind, Source, Store, Timestamp, selectors,
};
use ramflux_desktop_test::scale::{LARGE_CONTEXT, large_context, large_message_id};
use ramflux_desktop_test::{MockSource, SCENARIO_NAMES, ScenarioPlayer, load_fixture};

fn naive_timeline(store: &Store, context: &ContextId) -> Vec<MomentId> {
    let mut moments: Vec<_> = store.moments().filter(|m| &m.context_id == context).collect();
    moments.sort_by(|a, b| (a.at, &a.id).cmp(&(b.at, &b.id)));
    moments.into_iter().map(|m| m.id.clone()).collect()
}

fn naive_needs_me(store: &Store) -> Vec<MomentId> {
    let mut items: Vec<_> = store
        .moments()
        .filter(|m| m.lifecycle == Lifecycle::NeedsMe)
        .filter(|m| !selectors::is_task_owned_intervention(store, m))
        .map(|m| (store.needs_me_since(&m.id).unwrap_or(m.at), m.id.clone()))
        .collect();
    items.sort();
    // A task whose step waits on a surface needs the user at that surface.
    items
        .into_iter()
        .map(|(_, id)| {
            let moment = store.moment(&id).expect("moment");
            selectors::task_decision_surface(store, moment).cloned().unwrap_or(id)
        })
        .collect()
}

fn agrees(store: &Store, at: &str) {
    for context in store.contexts() {
        let indexed: Vec<MomentId> =
            selectors::timeline(store, &context.id).iter().map(|m| m.id.clone()).collect();
        assert_eq!(indexed, naive_timeline(store, &context.id), "{at}: timeline {}", context.id);
    }
    let attention: Vec<MomentId> =
        selectors::attention(store).into_iter().map(|i| i.moment_id).collect();
    assert_eq!(attention, naive_needs_me(store), "{at}: attention");
}

#[test]
fn indexes_match_a_full_scan_through_every_scenario() {
    for name in SCENARIO_NAMES.iter().chain(["s09_work_account"].iter()) {
        let mut player = ScenarioPlayer::new(&load_fixture(name).expect("fixture"));
        let mut step = 0;
        while player.step() {
            step += 1;
            agrees(player.store(), &format!("{name} step {step}"));
        }
        assert!(player.errors().is_empty(), "{name}: {:?}", player.errors());
    }
}

#[test]
fn evicted_bodies_are_delivered_again_on_request() {
    let scenario = large_context(300);
    let mut source = MockSource::new(&scenario);
    let mut store = Store::with_body_capacity(100);
    let apply = |source: &mut MockSource, store: &mut Store, at: i64| {
        for env in source.drain(Timestamp(scenario.epoch.millis() + at)) {
            store.apply(&env).expect("apply");
        }
    };
    apply(&mut source, &mut store, 0);
    assert_eq!(store.context_moment_count(&ContextId::new(LARGE_CONTEXT)), 300);
    let oldest = large_message_id(0);
    assert!(store.body(&oldest).is_none(), "evicted: only the newest 100 bodies stay");
    assert!(store.body(&large_message_id(299)).is_some());
    source.request_bodies(&[oldest.clone(), large_message_id(1)]);
    apply(&mut source, &mut store, 10);
    assert!(store.body(&oldest).is_some_and(|b| b.starts_with("Message 0:")));
    assert!(store.body(&large_message_id(1)).is_some());
    let row = store.moment(&oldest).expect("row");
    assert!(matches!(&row.kind, MomentKind::Message(p) if p.text.is_empty()), "rows stay metadata");
}
