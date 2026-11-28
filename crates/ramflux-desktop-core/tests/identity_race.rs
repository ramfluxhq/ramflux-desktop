// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! An acceptance in flight against a newer identity change: the outcome of
//! the old attempt never clears or hides the newer change.

mod common;

use common::*;
use ramflux_desktop_core::*;

fn feed() -> Feed {
    let mut feed = Feed::new();
    feed.ok(
        1,
        DomainEvent::ParticipantUpserted(participant("p-1", ParticipantKind::Human, "Chen", true)),
    );
    feed.ok(2, changed("A1", "A2"));
    feed
}

fn changed(previous: &str, new: &str) -> DomainEvent {
    DomainEvent::IdentityChanged {
        participant: ParticipantId::new("p-1"),
        username: None,
        previous_anchor: previous.to_owned(),
        new_anchor: new.to_owned(),
        ai_flag: false,
    }
}

fn failed(reason: AcceptFailure, attempted: Option<&str>) -> DomainEvent {
    DomainEvent::IdentityAcceptFailed {
        participant: ParticipantId::new("p-1"),
        reason,
        attempted_anchor: attempted.map(str::to_owned),
    }
}

fn pending(feed: &Feed) -> Option<String> {
    feed.store
        .participant(&ParticipantId::new("p-1"))
        .and_then(|p| p.pending_change.as_ref())
        .map(|c| c.new_anchor.clone())
}

#[test]
fn already_handled_for_an_old_anchor_keeps_the_newer_change() {
    let mut feed = feed();
    // A2 accept in flight; A3 arrives; the reply says nothing was pending.
    feed.ok(3, changed("A2", "A3"));
    feed.ok(4, failed(AcceptFailure::AlreadyHandled, Some("A2")));
    assert_eq!(pending(&feed).as_deref(), Some("A3"), "A3 stays pending and reviewable");
    assert!(
        feed.store.participant(&ParticipantId::new("p-1")).is_some_and(|p| p.identity.is_changed())
    );
    let failure = feed.store.identity_failure(&ParticipantId::new("p-1")).expect("failure");
    assert_eq!((failure.0, failure.2), (AcceptFailure::AlreadyHandled, Some("A2")));
}

#[test]
fn already_handled_for_the_pending_anchor_clears_it() {
    let mut feed = feed();
    feed.ok(3, failed(AcceptFailure::AlreadyHandled, Some("A2")));
    assert_eq!(pending(&feed), None);
}

#[test]
fn already_handled_without_an_anchor_clears_what_is_pending() {
    let mut feed = feed();
    feed.ok(3, failed(AcceptFailure::AlreadyHandled, None));
    assert_eq!(pending(&feed), None);
}

#[test]
fn other_outcomes_keep_the_newer_change() {
    for reason in [AcceptFailure::ChangedAgain, AcceptFailure::Failed] {
        let mut feed = feed();
        feed.ok(3, changed("A2", "A3"));
        feed.ok(4, failed(reason, Some("A2")));
        assert_eq!(pending(&feed).as_deref(), Some("A3"));
    }
    let mut feed = feed();
    feed.ok(3, changed("A2", "A3"));
    feed.ok(
        4,
        DomainEvent::IdentityAccepted {
            participant: ParticipantId::new("p-1"),
            new_anchor: Some("A2".to_owned()),
        },
    );
    assert_eq!(pending(&feed).as_deref(), Some("A3"));
}
