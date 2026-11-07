// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The one change of kind a moment may make: a "newer message type"
//! placeholder becomes the message it stood for.

mod common;

use common::*;
use ramflux_desktop_core::*;

fn placeholder(unsupported: Option<UnsupportedInfo>) -> MomentKind {
    MomentKind::Fallback(FallbackPayload {
        reason: "r".to_owned(),
        text: String::new(),
        unsupported,
    })
}

fn info() -> UnsupportedInfo {
    UnsupportedInfo {
        kind: "99".to_owned(),
        layer: "1".to_owned(),
        reason: "unknown_kind".to_owned(),
        raw_length: 1,
    }
}

fn store_with(kind: MomentKind) -> Feed {
    let mut feed = Feed::new();
    feed.ok(1, DomainEvent::ContextUpserted(context("c-1", false)));
    feed.ok(2, added(moment("m-1", "c-1", 5, kind, Lifecycle::Done)));
    feed
}

fn update(kind: MomentKind) -> DomainEvent {
    DomainEvent::MomentUpdated {
        moment_id: MomentId::new("m-1"),
        lifecycle: None,
        kind: Some(kind),
    }
}

#[test]
fn a_placeholder_becomes_the_message() {
    let mut feed = store_with(placeholder(Some(info())));
    feed.ok(3, update(message("now readable")));
    let stored = feed.store.moment(&MomentId::new("m-1")).expect("moment");
    assert!(matches!(stored.kind, MomentKind::Message(_)));
    assert_eq!(
        feed.store.bodies().get(&MomentId::new("m-1")).map(|t| t.to_string()),
        Some("now readable".to_owned())
    );
}

#[test]
fn a_message_never_becomes_a_placeholder() {
    let mut feed = store_with(message("text"));
    let result = feed.push(3, update(placeholder(Some(info()))));
    assert_eq!(result, Err(ApplyError::KindMismatch(MomentId::new("m-1"))));
}

#[test]
fn a_plain_fallback_cannot_change_kind() {
    let mut feed = store_with(placeholder(None));
    let result = feed.push(3, update(message("text")));
    assert_eq!(result, Err(ApplyError::KindMismatch(MomentId::new("m-1"))));
}
