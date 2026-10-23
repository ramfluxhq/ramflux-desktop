// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Fail-closed grant errors: the daemon's error frames on decisions (taken
//! from the daemon at the pinned product revision) become explicit grant
//! failures, and a refused approval is never sent again.

mod common;

use common::*;
use ramflux_desktop_core::{
    AccountId, DomainEvent, Effect, FailureCode, GrantFailure, GrantId, GrantState, Source as _,
    Timestamp, methods,
};
use ramflux_desktop_sdk::Scope;
use ramflux_desktop_sdk::plan::fetch::Fetch;
use ramflux_desktop_sdk::translate::Translator;
use serde_json::{Value, json};

const NOW: Timestamp = Timestamp(1_800_000_000_000);

/// A translator that has seen the fixture's pending approval `appr-1`.
fn translator() -> Translator {
    translator_with(frames()["mcp.approval.list"].clone())
}

fn translator_with(approvals: Value) -> Translator {
    let mut t = Translator::new(Scope::Account(AccountId::new("acct-main")), "/tmp/rfd.sock");
    let events = t.fetched(&Fetch::Approvals, &approvals, NOW).events;
    assert!(events.iter().any(|e| matches!(e, DomainEvent::GrantRequested(_))));
    t
}

/// The `(code, message)` of a recorded error frame.
fn frame(name: &str) -> (FailureCode, String) {
    let frame = &frames()[name];
    (
        FailureCode::Daemon(frame["code"].as_str().expect("code").to_owned()),
        frame["message"].as_str().expect("message").to_owned(),
    )
}

fn approve_body() -> Value {
    json!({ "grant_id": "appr-1", "args_hash": "h-123" })
}

/// Sends one approval and feeds the daemon's error frame back.
fn refuse(t: &mut Translator, name: &str) -> Vec<DomainEvent> {
    t.decision_sent(methods::GRANT_APPROVE, &approve_body());
    let (code, message) = frame(name);
    t.effect_failed(methods::GRANT_APPROVE, &approve_body(), &code, &message, NOW)
}

fn failed(events: &[DomainEvent]) -> Vec<GrantFailure> {
    events
        .iter()
        .filter_map(|e| match e {
            DomainEvent::GrantFailed { grant_id, failure } if grant_id.as_str() == "appr-1" => {
                Some(*failure)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_request_that_needs_a_signed_approval_fails_closed_once() {
    let mut t = translator();
    let events = refuse(&mut t, "grant.approve#source_error:remote_app");
    assert_eq!(failed(&events), [GrantFailure::SignedApprovalRequired]);
    assert!(refuse(&mut t, "grant.approve#source_error:remote_app").is_empty(), "reported once");
}

#[test]
fn approval_not_found_is_already_used_first_and_a_replay_after_a_decision() {
    let mut first = translator();
    assert_eq!(
        failed(&refuse(&mut first, "grant.approve#source_error:not_found")),
        [GrantFailure::AlreadyUsed]
    );

    let mut twice = translator();
    twice.decision_sent(methods::GRANT_APPROVE, &approve_body());
    assert_eq!(
        failed(&refuse(&mut twice, "grant.approve#source_error:not_found")),
        [GrantFailure::Replay],
        "a decision sent again for the same approval is a replay"
    );

    let mut decided = translator();
    let approved = decided.effect_done(methods::GRANT_APPROVE, &approve_body(), &json!({}), NOW);
    assert!(
        approved
            .iter()
            .any(|e| matches!(e, DomainEvent::GrantResolved { state: GrantState::Approved, .. }))
    );
    let (code, message) = frame("grant.deny#source_error:not_found");
    let body = json!({ "grant_id": "appr-1" });
    decided.decision_sent(methods::GRANT_DENY, &body);
    let events = decided.effect_failed(methods::GRANT_DENY, &body, &code, &message, NOW);
    assert_eq!(failed(&events), [GrantFailure::Replay]);
}

#[test]
fn a_used_up_approval_past_its_expiry_is_expired() {
    let mut list = frames()["mcp.approval.list"].clone();
    list["approvals"][0]["expires_at"] = json!(1_700_000_000);
    let mut t = translator_with(list);
    assert_eq!(
        failed(&refuse(&mut t, "grant.approve#source_error:not_found")),
        [GrantFailure::Expired]
    );
    let mut invalidated = translator();
    assert_eq!(
        failed(&refuse(&mut invalidated, "mcp.tool.started#source_error:invalidated")),
        [GrantFailure::Invalidated]
    );
    let mut signature = translator();
    assert_eq!(
        failed(&refuse(&mut signature, "mcp.approval.granted#source_error:signature")),
        [GrantFailure::SignatureRejected]
    );
}

#[test]
fn an_approval_bound_to_other_arguments_fails_closed() {
    let mut t = translator();
    let answer = frames()["grant.approve#rebound"].clone();
    let events = t.effect_done(methods::GRANT_APPROVE, &approve_body(), &answer, NOW);
    assert_eq!(failed(&events), [GrantFailure::ArgumentsMismatch]);

    let mut pushed = translator();
    let mut record = frames()["mcp.approval.list"]["approvals"][0].clone();
    record["details"]["arguments_hash"] = json!("h-other");
    let events = pushed.pushed("mcp.approval.request", &record);
    assert_eq!(failed(&events), [GrantFailure::ArgumentsMismatch]);
    let same = frames()["mcp.approval.list"]["approvals"][0].clone();
    assert!(translator().pushed("mcp.approval.request", &same).is_empty());
}

#[test]
fn transport_errors_and_other_methods_close_nothing() {
    let mut t = translator();
    t.decision_sent(methods::GRANT_APPROVE, &approve_body());
    let lost = t.effect_failed(
        methods::GRANT_APPROVE,
        &approve_body(),
        &FailureCode::Transport,
        "connection reset",
        NOW,
    );
    assert!(lost.is_empty(), "an unknown outcome is not a failure");
    let (code, message) = frame("grant.approve#source_error:not_found");
    let other = t.effect_failed(methods::GRANT_REVOKE, &approve_body(), &code, &message, NOW);
    assert!(other.is_empty());
}

#[test]
fn a_refused_approval_shows_as_failed_and_is_never_sent_again() {
    let path = socket_path("grant-closed");
    let (code, message) = frame("grant.approve#source_error:remote_app");
    let FailureCode::Daemon(code) = code else { unreachable!() };
    let code: &'static str = Box::leak(code.into_boxed_str());
    let fake = FakeBus::start(&path, move |method, _, _| match method {
        "grant.approve" => Reply::Error(code, message.clone()),
        _ => fixture(method),
    });
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("approval", |s, _| s.grant(&GrantId::new("appr-1")).is_some());
    account.source.submit(Effect::Call {
        method: methods::GRANT_APPROVE.to_owned(),
        body: json!({ "grant_id": "appr-1", "args_hash": "h-123", "proposed_by": "Human", "confirmed_by": null }),
    });
    account.until("failed closed", |s, _| {
        s.grant(&GrantId::new("appr-1")).is_some_and(|g| g.failure.is_some())
    });
    let grant = account.store.grant(&GrantId::new("appr-1")).expect("grant");
    assert_eq!(grant.failure, Some(GrantFailure::SignedApprovalRequired));
    assert_eq!(grant.state, GrantState::Pending);
    assert_eq!(account.store.failures().len(), 1);
    // Polls and re-snapshots keep running; the approval is never retried.
    account.settle(700);
    let approvals = fake.seen().into_iter().filter(|s| s.method == "grant.approve").count();
    assert_eq!(approvals, 1, "an approval is never sent again by the adapter");
    assert!(account.store.rejections().is_empty(), "{:?}", account.store.rejections());
    fake.shutdown();
}
