// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Shared helpers for the adapter tests.

#![allow(dead_code)]

pub mod fake_bus;

use std::time::{Duration, Instant};

use ramflux_desktop_core::{EventEnvelope, Source, Store, Timestamp};
use ramflux_desktop_sdk::{Deadlines, SdkConfig, SdkHub, SdkSource};

// Not every test target uses every helper (the live tests use none of the fake).
#[allow(unused_imports)]
pub use fake_bus::{FakeBus, Reply, frames, socket_path};

/// Fast timings for tests.
pub fn config(path: &std::path::Path) -> SdkConfig {
    SdkConfig {
        connect_timeout: Duration::from_secs(2),
        deadlines: Deadlines::uniform(Duration::from_secs(3)),
        poll_fast: Duration::from_millis(80),
        poll_slow: Duration::from_millis(300),
        reconnect_backoff: Duration::from_millis(150),
        ..SdkConfig::new(path)
    }
}

/// The fixture answer for `method`, or an empty object.
pub fn fixture(method: &str) -> Reply {
    Reply::Ok(frames().get(method).cloned().unwrap_or_else(|| serde_json::json!({})))
}

/// A store fed by one source, with every envelope kept.
pub struct Fed {
    pub source: SdkSource,
    pub store: Store,
    pub envelopes: Vec<EventEnvelope>,
}

impl Fed {
    pub fn new(source: SdkSource) -> Self {
        Self { source, store: Store::new(), envelopes: Vec::new() }
    }

    /// Applies what has arrived.
    pub fn pump(&mut self) {
        for env in self.source.drain(Timestamp(0)) {
            let _ = self.store.apply(&env);
            self.envelopes.push(env);
        }
    }

    /// Pumps until `done` holds. The wall-clock limit only bounds a failure
    /// (a busy machine just takes longer); nothing is asserted about speed.
    pub fn until(&mut self, what: &str, done: impl Fn(&Store, &[EventEnvelope]) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.pump();
            if done(&self.store, &self.envelopes) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for: {what}");
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    /// Pumps for `ms` milliseconds.
    pub fn settle(&mut self, ms: u64) {
        let deadline = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < deadline {
            self.pump();
            std::thread::sleep(Duration::from_millis(15));
        }
        self.pump();
    }

    /// Names of every event received, in order.
    pub fn names(&self) -> Vec<String> {
        self.envelopes
            .iter()
            .map(|e| {
                format!("{:?}", e.event)
                    .split(['(', ' ', '{'])
                    .next()
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect()
    }
}

/// The directories a translator builds from the fixture's account answers
/// (nodes, account status, devices, identities, groups, conversations,
/// objects): what the planner reads routes from.
pub fn directory() -> ramflux_desktop_sdk::directory::Directory {
    use ramflux_desktop_sdk::plan::fetch::{Fetch, Paged};
    use ramflux_desktop_sdk::translate::Translator;
    let account = ramflux_desktop_core::AccountId::new("acct-main");
    let mut translator =
        Translator::new(ramflux_desktop_sdk::Scope::Account(account.clone()), "/tmp/rfd.sock");
    let frames = frames();
    for fetch in [
        Fetch::NodeList,
        Fetch::AccountStatus(account),
        Fetch::DeviceList,
        Fetch::first(Paged::Identities),
        Fetch::first(Paged::Groups),
        Fetch::first(Paged::Conversations),
        Fetch::first(Paged::Objects),
    ] {
        let method = fetch.request(Some("acct-main")).method;
        let _ = translator.fetched(&fetch, &frames[&method], Timestamp(0));
    }
    translator.directory().clone()
}

/// Starts a hub on `path`.
pub fn hub(path: &std::path::Path) -> SdkHub {
    SdkHub::start(config(path)).expect("bridge")
}
