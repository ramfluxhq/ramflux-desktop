// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Runs UI integration tests in a test binary that has its own `main`
//! (`harness = false`), reporting in libtest's format.
//!
//! A binary that must also act as something else when started with other
//! arguments (the stand-in daemon, see `stand_in`) cannot use libtest,
//! which rejects those arguments. Each test gets what `#[gpui_kit::test]`
//! gives it: a fresh `TestAppContext` on a deterministic test dispatcher,
//! run until parked and shut down afterwards. Tests run one at a time.
//!
//! Name filters, `--exact`, `--skip <name>`, `--list` and `--ignored` work;
//! other flags are accepted and ignored. A failure exits with 101.

use gpui_kit::{TestAppContext, TestDispatcher};

/// A named UI test.
pub type Test = (&'static str, fn(&mut TestAppContext));

/// Flags of libtest that take a value.
const VALUED: [&str; 6] =
    ["--test-threads", "--format", "--color", "--logfile", "--shuffle-seed", "-Z"];

/// Runs one test as `#[gpui_kit::test]` would.
fn run_one(name: &'static str, test: fn(&mut TestAppContext)) {
    gpui_kit::run_test(
        1,
        &[],
        0,
        &mut |dispatcher: TestDispatcher, _seed| {
            let mut cx = TestAppContext::build(dispatcher.clone(), Some(name));
            let _entity_refcounts = cx.app.borrow().ref_counts_drop_handle();
            test(&mut cx);
            cx.run_until_parked();
            cx.update(|cx| {
                cx.background_executor().forbid_parking();
                cx.quit();
            });
            cx.run_until_parked();
            drop(cx);
            dispatcher.drain_tasks();
            drop(dispatcher);
        },
        None,
    );
}

/// Runs the chosen `tests` with this process's arguments, then exits.
pub fn main(tests: &[Test]) -> ! {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut filters = Vec::new();
    let mut skips = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--skip" {
            skips.extend(iter.next().cloned());
        } else if VALUED.contains(&arg.as_str()) {
            iter.next();
        } else if !arg.starts_with('-') {
            filters.push(arg.clone());
        }
    }
    let exact = args.iter().any(|a| a == "--exact");
    let matches = |name: &str, pattern: &str| {
        if exact { name == pattern } else { name.contains(pattern) }
    };
    // None of these tests is ignored: `--ignored` alone runs nothing.
    let ignored_only = args.iter().any(|a| a == "--ignored");
    let chosen: Vec<&Test> = tests
        .iter()
        .filter(|(name, _)| filters.is_empty() || filters.iter().any(|f| matches(name, f)))
        .filter(|(name, _)| !skips.iter().any(|s| matches(name, s)))
        .filter(|_| !ignored_only)
        .collect();
    if args.iter().any(|a| a == "--list") {
        for (name, _) in &chosen {
            println!("{name}: test");
        }
        std::process::exit(0);
    }
    println!("\nrunning {} tests", chosen.len());
    let mut failed = Vec::new();
    for (name, test) in &chosen {
        let ok = std::panic::catch_unwind(|| run_one(name, *test)).is_ok();
        println!("test {name} ... {}", if ok { "ok" } else { "FAILED" });
        if !ok {
            failed.push(*name);
        }
    }
    if !failed.is_empty() {
        println!("\nfailures:");
        for name in &failed {
            println!("    {name}");
        }
    }
    println!(
        "\ntest result: {}. {} passed; {} failed; 0 ignored; 0 measured; {} filtered out\n",
        if failed.is_empty() { "ok" } else { "FAILED" },
        chosen.len() - failed.len(),
        failed.len(),
        tests.len() - chosen.len()
    );
    std::process::exit(if failed.is_empty() { 0 } else { 101 });
}
