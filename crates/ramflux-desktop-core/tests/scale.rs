// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Scale benchmarks as timed tests: the store and its selectors stay fast
//! with real data volumes.
//!
//! The data set is deterministic: 200 contexts, 100 000 moments (one
//! context holds 20 000 of them), 1 000 of them waiting on the user across
//! the 200 contexts. Each benchmark prints its time
//! (`cargo test -p ramflux-desktop-core --test scale -- --nocapture`). The
//! results (counts, bounded bodies) are always checked. The time bounds
//! (generous, sized for unoptimized test builds) depend on how busy the
//! machine is, so they are asserted only on request, keeping the default
//! `cargo test` deterministic:
//!
//! ```sh
//! RAMFLUX_PERF_ASSERT=1 cargo test -p ramflux-desktop-core --test scale -- --nocapture
//! ```

mod common;

use std::time::{Duration, Instant};

use common::*;
use ramflux_desktop_core::*;

const CONTEXTS: usize = 200;
const BIG: &str = "c-big";
const BIG_MOMENTS: usize = 20_000;
const MOMENTS: usize = 100_000;
const NEEDS_ME: usize = 1_000;

fn context_id(n: usize) -> String {
    format!("c-{n:03}")
}

/// Every envelope of the data set, in order.
fn envelopes() -> Vec<EventEnvelope> {
    let mut events = Vec::with_capacity(MOMENTS + CONTEXTS + 2);
    let mut big = context(BIG, true);
    big.participants = vec![ParticipantId::new("p-ada"), ParticipantId::new("p-bo")];
    events.push(DomainEvent::ContextUpserted(big));
    for n in 0..CONTEXTS {
        let mut ctx = context(&context_id(n), n % 50 == 0);
        ctx.participants = vec![ParticipantId::new("p-ada"), ParticipantId::new(format!("p-{n}"))];
        events.push(DomainEvent::ContextUpserted(ctx));
    }
    let spread = MOMENTS - BIG_MOMENTS;
    for n in 0..MOMENTS {
        let (ctx, at) = if n < BIG_MOMENTS {
            (BIG.to_owned(), i64::try_from(n).unwrap_or(0))
        } else {
            let k = n - BIG_MOMENTS;
            (context_id(k % CONTEXTS), i64::try_from(k / CONTEXTS).unwrap_or(0))
        };
        // Every 80th row of each spread context waits on the user: five in
        // each of the 200 contexts, 1 000 in all.
        let rows = spread / CONTEXTS;
        let every = rows / (NEEDS_ME / CONTEXTS);
        let needs_me = n >= BIG_MOMENTS && ((n - BIG_MOMENTS) / CONTEXTS).is_multiple_of(every);
        let lifecycle = if needs_me { Lifecycle::NeedsMe } else { Lifecycle::Done };
        let text = format!("message {n} with a sentence of ordinary length for the timeline");
        events.push(added(moment(&format!("m-{n:06}"), &ctx, at, message(&text), lifecycle)));
    }
    events
        .into_iter()
        .enumerate()
        .map(|(i, event)| {
            let seq = u64::try_from(i).unwrap_or(0) + 1;
            EventEnvelope::new(seq, Timestamp(i64::try_from(i).unwrap_or(0)), event)
        })
        .collect()
}

fn loaded() -> Store {
    let mut store = Store::new();
    for env in &envelopes() {
        store.apply(env).expect("apply");
    }
    store
}

/// Best of `runs` timings of `f`.
fn best<R>(runs: usize, mut f: impl FnMut() -> R) -> (Duration, R) {
    let mut best = Duration::MAX;
    let mut last = None;
    for _ in 0..runs {
        let started = Instant::now();
        let result = f();
        best = best.min(started.elapsed());
        last = Some(result);
    }
    (best, last.expect("at least one run"))
}

/// Whether wall-clock bounds are asserted (`RAMFLUX_PERF_ASSERT=1`).
fn perf_asserted() -> bool {
    std::env::var("RAMFLUX_PERF_ASSERT").is_ok_and(|v| v == "1")
}

fn report(name: &str, took: Duration, bound: Duration) {
    println!("BENCH {name}: {:.3} ms (bound {} ms)", took.as_secs_f64() * 1e3, bound.as_millis());
    if perf_asserted() {
        assert!(took <= bound, "{name} took {took:?}, over its bound {bound:?}");
    }
}

#[test]
fn applying_100k_events_stays_fast_and_bodies_stay_bounded() {
    let all = envelopes();
    let (took, store) = best(1, || {
        let mut store = Store::new();
        for env in &all {
            store.apply(env).expect("apply");
        }
        store
    });
    report("apply_100k_events", took, Duration::from_secs(10));
    assert_eq!(store.moments().count(), MOMENTS);
    assert!(store.bodies().len() <= MAX_BODIES, "bodies are bounded");
    assert!(store.body(&MomentId::new(format!("m-{:06}", MOMENTS - 1))).is_some());
}

#[test]
fn a_20k_timeline_and_a_context_switch_cost_their_own_size() {
    let store = loaded();
    let (big_took, timeline) = best(5, || selectors::timeline(&store, &ContextId::new(BIG)).len());
    report("timeline_20k", big_took, Duration::from_millis(200));
    assert_eq!(timeline, BIG_MOMENTS);
    let small = ContextId::new(context_id(7));
    let (took, count) = best(5, || {
        // What showing another context reads: its timeline, its members,
        // its attention count.
        let rows = selectors::timeline(&store, &small).len();
        let members = selectors::participants(&store, &small).len();
        let needs_me = selectors::needs_me_count(&store, &small);
        rows + members + needs_me
    });
    report("context_switch_400", took, Duration::from_millis(10));
    assert!(count >= (MOMENTS - BIG_MOMENTS) / CONTEXTS);
    // Cost follows the context's own size (400 versus 20 000 moments), not
    // the 100 000 in the store.
    if perf_asserted() {
        assert!(took * 5 < big_took, "a small context costs {took:?}, the big one {big_took:?}");
    }
}

#[test]
fn attention_over_1k_needs_me_in_200_contexts_is_fast() {
    let store = loaded();
    let (took, items) = best(5, || selectors::attention(&store));
    report("attention_1k", took, Duration::from_millis(25));
    assert_eq!(items.len(), NEEDS_ME);
    let contexts: std::collections::BTreeSet<_> = items.iter().map(|i| &i.context_id).collect();
    assert_eq!(contexts.len(), CONTEXTS, "spread across every context");
    let (took, stack) = best(5, || selectors::context_stack(&store));
    report("context_stack_1k", took, Duration::from_millis(25));
    assert_eq!(stack.len(), CONTEXT_STACK_CAP);
}
