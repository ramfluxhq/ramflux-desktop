// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The demo rehearsal harness on two fake daemons sharing one fake node:
//! every step passes on a node that behaves, a node whose key is not the
//! expected one is never confirmed (and every later step is skipped), and
//! the harness's own pieces (step table, key check) hold.

mod demo_fake;
mod rehearsal;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use demo_fake::world::World;
use demo_fake::{FakeDaemon, Shared};
use gpui_kit::TestAppContext;
use ramflux_desktop_core::NodeId;
use ramflux_desktop_sdk::SdkConfig;
use ramflux_desktop_ui::shell::{ShellOptions, SourceChoice};
use rehearsal::config::{
    DemoConfig, Limits, fingerprint_matches, fingerprint_of, passphrase, run_id,
};
use rehearsal::flow::{self, Demo, Side};
use rehearsal::steps::{Outcome, Steps};

const URL: &str = "https://demo.node.invalid";
const KEY: &str = "MCowBQYDK2VwAyEA-fake-demo-node-key";

fn options(socket: &std::path::Path) -> ShellOptions {
    let config = SdkConfig {
        deadlines: ramflux_desktop_sdk::Deadlines::uniform(Duration::from_secs(3)),
        poll_fast: Duration::from_millis(80),
        poll_slow: Duration::from_millis(300),
        reconnect_backoff: Duration::from_millis(150),
        ..SdkConfig::new(socket)
    };
    ShellOptions { source: SourceChoice::Rfd(config), ..ShellOptions::for_scenario("", true) }
}

fn config(key: &str) -> DemoConfig {
    DemoConfig {
        node_url: URL.to_owned(),
        node_key: key.to_owned(),
        run_id: "20261023-120000".to_owned(),
        shots: None,
        limits: Limits::FAKE,
    }
}

/// A scratch folder for the attachment, removed on drop.
struct Work(std::path::PathBuf);

impl Work {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rf-f9-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("work dir");
        Self(dir)
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Runs the rehearsal on a fresh fake node; `check` sees the finished demo.
fn rehearse(
    cx: &mut TestAppContext,
    name: &str,
    expected_key: &str,
    check: impl FnOnce(&Steps, &mut Demo<'_, TestAppContext, FakeDaemon>, &Shared),
) -> Steps {
    cx.update(|cx| {
        gpui_kit::init(cx);
        ramflux_desktop_ui::init(cx);
    });
    let world: Shared = Arc::new(Mutex::new(World::new(URL, KEY)));
    let alice = FakeDaemon::start(&world, 0, &format!("{name}-a"));
    let bob = FakeDaemon::start(&world, 1, &format!("{name}-b"));
    let work = Work::new(name);
    let mut demo = Demo::new(
        cx,
        config(expected_key),
        Box::new(options),
        Side::new("a", "Alice", alice),
        Side::new("b", "Bob", bob),
        work.0.clone(),
    );
    let steps = flow::run(&mut demo);
    println!("{}", steps.markdown());
    check(&steps, &mut demo, &world);
    steps
}

#[gpui_kit::test]
fn every_step_passes_on_a_node_that_behaves(cx: &mut TestAppContext) {
    let steps = rehearse(cx, "f9-ok", KEY, |steps, demo, _| {
        assert!(steps.all_passed(), "{}", steps.markdown());
        assert_eq!(demo.node(), Some(&NodeId::new("demo-node")));
        let [alice, bob] = demo.sides();
        let (alice, bob) = (alice.facts.clone().expect("alice"), bob.facts.clone().expect("bob"));
        assert_ne!(alice.principal, bob.principal);
        assert!(!alice.commitment.is_empty() && !bob.target.is_empty());
    });
    assert_eq!(steps.rows().len(), 7);
    let notes = |n: u8| steps.rows()[usize::from(n - 1)].notes.join("; ");
    assert!(notes(2).contains("confirmed with the hold"), "{}", notes(2));
    assert!(notes(2).contains("the daemon pinned that key"), "{}", notes(2));
    assert!(notes(4).contains("safety number verified"), "{}", notes(4));
    assert!(notes(5).contains("reference CLI"), "the opener's path is stated: {}", notes(5));
    assert!(notes(5).contains("Delivered"), "{}", notes(5));
    assert!(notes(6).contains("attachment not fetched"), "{}", notes(6));
    assert!(notes(7).contains("locked again"), "{}", notes(7));
    assert!(notes(7).contains("history kept"), "{}", notes(7));
}

#[gpui_kit::test]
fn a_node_with_another_key_is_never_confirmed(cx: &mut TestAppContext) {
    let steps = rehearse(cx, "f9-key", "MCowBQYDK2VwAyEA-some-other-key", |_, demo, world| {
        assert_eq!(demo.node(), None);
        let world = world.lock().expect("world");
        assert_eq!(world.node_key(0, "demo-node"), None, "daemon A pinned nothing");
        assert_eq!(world.node_key(1, "demo-node"), None, "daemon B pinned nothing");
    });
    assert_eq!(steps.rows()[0].outcome, Outcome::Pass);
    match &steps.rows()[1].outcome {
        Outcome::Fail(why) => assert!(why.contains("does not match the expected key"), "{why}"),
        other => panic!("step 2 should fail: {other:?}"),
    }
    assert!(
        steps.rows()[2..].iter().all(|r| matches!(r.outcome, Outcome::Skipped(_))),
        "{}",
        steps.markdown()
    );
}

#[gpui_kit::test]
fn the_expected_key_may_be_given_as_its_fingerprint(cx: &mut TestAppContext) {
    let steps = rehearse(cx, "f9-fp", &fingerprint_of(KEY), |_, _, _| {});
    assert!(steps.passed(2), "{}", steps.markdown());
}

// The harness's own pieces.

#[test]
fn a_fingerprint_matches_its_key_or_itself_and_nothing_else() {
    let shown = fingerprint_of("MCowBQYDK2VwAyEA-demo-key");
    assert_eq!(shown.len(), 64, "a sha-256 hex digest: {shown}");
    assert!(fingerprint_matches("MCowBQYDK2VwAyEA-demo-key", &shown));
    assert!(fingerprint_matches(&shown.to_uppercase(), &shown), "the fingerprint itself");
    assert!(!fingerprint_matches("MCowBQYDK2VwAyEA-other-key", &shown));
    assert!(!fingerprint_matches("MCowBQYDK2VwAyEA-demo-key", ""), "no fingerprint shown");
}

#[test]
fn passphrases_are_fresh() {
    let (one, two) = (passphrase(), passphrase());
    assert_eq!(one.len(), 36);
    assert_ne!(one, two);
}

#[test]
fn run_ids_are_short_and_path_safe() {
    let id = run_id();
    assert_eq!(id.len(), 15, "{id}");
    assert!(id.chars().all(|c| c.is_ascii_digit() || c == '-'));
}

#[test]
fn a_failed_step_skips_the_steps_that_need_it_and_the_table_says_so() {
    let mut steps = Steps::default();
    assert!(steps.run(1, "start", &[], |notes| {
        notes.add("two daemons");
        Ok(())
    }));
    assert!(!steps.run(2, "node", &[1], |_| Err("fingerprint | mismatch".to_owned())));
    let mut ran = false;
    assert!(!steps.run(3, "accounts", &[1, 2], |_| {
        ran = true;
        Ok(())
    }));
    assert!(!ran, "a step whose prerequisite failed never runs");
    assert!(steps.run(4, "independent", &[1], |_| Ok(())));
    assert!(!steps.all_passed());
    let table = steps.markdown();
    assert!(table.contains("| 1 | start | pass |"), "{table}");
    assert!(table.contains("| two daemons |"), "{table}");
    assert!(table.contains("| 2 | node | FAIL |"), "{table}");
    assert!(table.contains("fingerprint \\| mismatch"), "cells are escaped: {table}");
    assert!(table.contains("| 3 | accounts | skipped |"), "{table}");
    assert!(table.contains("needs step 2"), "{table}");
    assert_eq!(steps.rows().len(), 4);
}
