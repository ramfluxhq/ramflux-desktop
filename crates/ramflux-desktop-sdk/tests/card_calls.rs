// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Card runtime calls against a fake daemon: the releases an account sends
//! just before its source goes away all reach the daemon, although card calls
//! otherwise run one at a time.

mod common;

use std::time::{Duration, Instant};

use common::*;
use ramflux_desktop_core::{AccountId, Effect, Source, SourceStatus, methods};
use serde_json::json;

#[test]
fn every_dispose_sent_before_the_source_drops_reaches_the_daemon() {
    let path = socket_path("card-disposes");
    // The first dispose is answered slowly, so the others wait behind it in
    // the one-at-a-time gate when the source goes away.
    let fake = FakeBus::start(&path, |method, _, _| match method {
        "card.session.dispose" => Reply::Slow(
            Duration::from_millis(300),
            json!({ "result": "disposed", "render_allowed": true, "fallback_text": "" }),
        ),
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("connected", |s, _| s.source_status() == SourceStatus::Connected);
    for (surface, session) in [("s-1", 1), ("s-2", 2), ("s-3", 3)] {
        account.source.submit(Effect::Call {
            method: methods::CARD_SESSION_DISPOSE.to_owned(),
            body: json!({ "surface_id": surface, "session": session }),
        });
    }
    drop(account);
    let deadline = Instant::now() + WAIT_LIMIT;
    let disposed = || {
        let mut sessions: Vec<u64> = fake
            .seen()
            .into_iter()
            .filter(|s| s.method == "card.session.dispose")
            .filter_map(|s| s.body["session"].as_u64())
            .collect();
        sessions.sort_unstable();
        sessions
    };
    while disposed() != [1, 2, 3] {
        assert!(Instant::now() < deadline, "disposes seen: {:?}", disposed());
        std::thread::sleep(Duration::from_millis(20));
    }
}
