// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Moment order key and the transient typing store (S10c c02b).

mod common;

use common::*;
use ramflux_desktop_core::*;
use serde::Serialize;
use serde_json::json;
use std::marker::PhantomData;

/// Whether `T` implements `Serialize`: the inherent method wins when the
/// bound holds, the trait default answers otherwise.
struct Probe<T>(PhantomData<T>);
trait NotSerializable {
    fn serializable() -> bool {
        false
    }
}
impl<T> NotSerializable for Probe<T> {}
impl<T: Serialize> Probe<T> {
    // Only reachable for a serializable `T`, which is what the test forbids.
    #[allow(dead_code)]
    fn serializable() -> bool {
        true
    }
}

fn key(hlc: u64, author: &str, device_sequence: u64) -> OrderKey {
    OrderKey { hlc, author: author.to_owned(), device_sequence }
}

#[test]
fn moment_order_and_typing_not_exported() {
    // Order: hlc, then author, then device sequence; display time never sorts.
    let mut keys = vec![key(2, "a", 1), key(1, "b", 9), key(1, "a", 2), key(1, "a", 1)];
    keys.sort();
    assert_eq!(keys, vec![key(1, "a", 1), key(1, "a", 2), key(1, "b", 9), key(2, "a", 1)]);

    // A moment without an order key serializes exactly as before and an old
    // JSON moment decodes to `None`; a keyed moment round-trips.
    let plain = moment("m-1", "c-1", 5, message("hi"), Lifecycle::Done);
    let plain_json = serde_json::to_value(&plain).expect("encode");
    assert!(plain_json.get("order").is_none(), "no order key, no field");
    let back: Moment = serde_json::from_value(plain_json).expect("old shape decodes");
    assert_eq!(back.order, None);
    let mut keyed = plain.clone();
    keyed.order = Some(key(7, "dev-1", 3));
    let value = serde_json::to_value(&keyed).expect("encode");
    assert_eq!(value["order"], json!({"hlc": 7, "author": "dev-1", "device_sequence": 3}));
    assert_eq!(serde_json::from_value::<Moment>(value).expect("decode"), keyed);

    // Typing: kept outside the store, with no way into a snapshot.
    assert!(!Probe::<TypingStore>::serializable(), "export has no typing");
    let mut feed = Feed::new();
    feed.ok(1, DomainEvent::ContextUpserted(context("c-1", false)));
    feed.ok(2, added(plain));
    let before = export::timeline_text(&feed.store, &ContextId::new("c-1"));
    let mut typing = TypingStore::new();
    typing.observe(
        &ContextId::new("c-1"),
        &ParticipantId::new("p-typist"),
        Timestamp(10),
        Timestamp(900),
    );
    assert_eq!(typing.typing(&ContextId::new("c-1"), Timestamp(10)).len(), 1);
    let after = export::timeline_text(&feed.store, &ContextId::new("c-1"));
    assert_eq!(before, after, "export has no typing");
    assert!(after.iter().all(|line| !line.contains("typist")), "export has no typing");
}

fn ctx() -> ContextId {
    ContextId::new("c-1")
}
fn who() -> ParticipantId {
    ParticipantId::new("p-1")
}

#[test]
fn typing_hidden_exactly_at_expires_at() {
    let mut typing = TypingStore::new();
    typing.observe(&ctx(), &who(), Timestamp(1_000), Timestamp(2_000));
    assert_eq!(typing.typing(&ctx(), Timestamp(1_999)).len(), 1, "visible before expires_at");
    assert!(typing.typing(&ctx(), Timestamp(2_000)).is_empty(), "hidden at expires_at");
}

#[test]
fn typing_expired_on_receipt_is_not_stored() {
    let mut typing = TypingStore::new();
    typing.observe(&ctx(), &who(), Timestamp(1_000), Timestamp(1_000));
    assert!(typing.is_empty(), "expires_at == now: no entry");
    typing.observe(&ctx(), &who(), Timestamp(1_000), Timestamp(999));
    assert!(typing.is_empty(), "expires_at < now: no entry");
    typing.observe(&ctx(), &who(), Timestamp(1_000), Timestamp(1_001));
    assert_eq!(typing.typing(&ctx(), Timestamp(1_000)).len(), 1, "now + 1 ms: shown");
}

#[test]
fn typing_is_capped_five_seconds_after_receipt() {
    let mut typing = TypingStore::new();
    typing.observe(&ctx(), &who(), Timestamp(1_000), Timestamp(7_000));
    assert_eq!(typing.typing(&ctx(), Timestamp(5_999)).len(), 1);
    assert!(typing.typing(&ctx(), Timestamp(6_000)).is_empty(), "hidden at receipt + 5 s");
    typing.expire(Timestamp(6_000));
    assert!(typing.is_empty(), "entry removed");
}

#[test]
fn typing_duplicate_never_extends_the_deadline() {
    let mut typing = TypingStore::new();
    typing.observe(&ctx(), &who(), Timestamp(1_000), Timestamp(2_000));
    typing.observe(&ctx(), &who(), Timestamp(1_500), Timestamp(9_000));
    assert!(typing.typing(&ctx(), Timestamp(2_000)).is_empty(), "stored deadline kept");
    typing.expire(Timestamp(2_000));
    assert!(typing.is_empty(), "expired entry pruned");
}

#[test]
fn typing_stop_and_clear_leave_nothing() {
    let mut typing = TypingStore::new();
    typing.observe(&ctx(), &who(), Timestamp(1_000), Timestamp(3_000));
    typing.observe(&ctx(), &ParticipantId::new("p-2"), Timestamp(1_000), Timestamp(3_000));
    typing.stop(&ctx(), &who());
    assert_eq!(typing.typing(&ctx(), Timestamp(1_000)), vec![&ParticipantId::new("p-2")]);
    typing.clear();
    assert!(typing.is_empty(), "cleared, not only hidden");
}

#[test]
fn typing_replay_after_the_display_cap_stays_hidden() {
    let mut typing = TypingStore::new();
    typing.observe(&ctx(), &who(), Timestamp(1_000), Timestamp(7_000));
    assert!(typing.typing(&ctx(), Timestamp(6_000)).is_empty(), "hidden at receipt + 5 s");
    // The identical notice replayed at the cap, before it expired itself.
    typing.observe(&ctx(), &who(), Timestamp(6_000), Timestamp(7_000));
    assert!(
        typing.typing(&ctx(), Timestamp(6_000)).is_empty(),
        "duplicate must not resurrect capped notice"
    );
    assert!(typing.is_empty(), "no visible entry");
    // Once the notice expired itself a new one is shown again.
    typing.observe(&ctx(), &who(), Timestamp(7_000), Timestamp(8_000));
    assert_eq!(typing.typing(&ctx(), Timestamp(7_000)).len(), 1);
}

#[test]
fn typing_stop_lets_a_new_notice_show() {
    let mut typing = TypingStore::new();
    typing.observe(&ctx(), &who(), Timestamp(1_000), Timestamp(3_000));
    typing.stop(&ctx(), &who());
    typing.observe(&ctx(), &who(), Timestamp(1_500), Timestamp(3_000));
    assert_eq!(typing.typing(&ctx(), Timestamp(1_500)).len(), 1, "stop forgets the old notice");
}

#[test]
fn moment_bytes_match_the_parent_writer_and_keyed_bytes_round_trip() {
    let plain = moment("m-1", "c-1", 5, message("hi"), Lifecycle::Done);
    let bytes = serde_json::to_string(&plain).expect("encode");
    assert_eq!(bytes, PARENT_GOLDEN, "an unkeyed moment encodes exactly as before the field");
    let mut keyed = plain;
    keyed.order = Some(key(7, "dev-1", 3));
    let keyed_bytes = serde_json::to_string(&keyed).expect("encode");
    let back: Moment = serde_json::from_str(&keyed_bytes).expect("decode");
    assert_eq!(
        serde_json::to_string(&back).expect("encode again"),
        keyed_bytes,
        "decode then encode keeps the bytes"
    );
}

/// A plain moment as the writer before `Moment.order` produced it.
const PARENT_GOLDEN: &str = r#"{"id":"m-1","context_id":"c-1","origin":"p-ada","at":5,"kind":{"Message":{"text":"hi","receipt":"Sent","tombstone":false,"attachment":null,"franking":null,"reply_to":null,"forward":null,"revision":0,"reactions":[],"spans":[],"media":[],"delete_scope":null}},"lifecycle":"Done"}"#;
