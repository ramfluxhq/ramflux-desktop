// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The translator's view of call rows and media sessions: each sequence
//! number is reported once, a row for a conversation not listed yet waits
//! for the listing it asks for, a malformed row is dropped, and a media
//! session is read with its secrets wiped from the answer.

mod common;

use common::*;
use ramflux_desktop_core::{AccountId, CallEndReason, CallId, CallState, DomainEvent, Timestamp};
use ramflux_desktop_sdk::plan::fetch::{Fetch, Paged};
use ramflux_desktop_sdk::translate::Translator;
use ramflux_desktop_sdk::translate::calls::call_media;
use serde_json::{Value, json};

fn translator() -> Translator {
    let account = AccountId::new("acct-main");
    let mut translator =
        Translator::new(ramflux_desktop_sdk::Scope::Account(account.clone()), "/tmp/rfd.sock");
    let frames = frames();
    for fetch in [Fetch::AccountStatus(account), Fetch::first(Paged::Conversations)] {
        let method = fetch.request(Some("acct-main")).method;
        let _ = translator.fetched(&fetch, &frames[&method], Timestamp(0));
    }
    translator
}

fn row(conversation: &str, state: &str, seq: u64) -> Value {
    json!({
        "call_id": "call-1", "conversation_id": conversation, "peer_id": "principal-ana",
        "direction": "outgoing", "state": state, "end_reason": null, "seq": seq,
        "created_at": 1_790_000_000, "answered_at": null, "ended_at": null,
    })
}

fn calls(events: &[DomainEvent]) -> Vec<(CallState, u64)> {
    events
        .iter()
        .filter_map(|e| match e {
            DomainEvent::CallUpserted(call) => Some((call.state, call.version)),
            _ => None,
        })
        .collect()
}

#[test]
fn each_sequence_number_is_reported_once() {
    let mut translator = translator();
    let first = translator.call_row(&row("dm-ana", "outgoing_ringing", 1));
    assert_eq!(calls(&first.events), [(CallState::OutgoingRinging, 1)]);
    let DomainEvent::CallUpserted(call) = &first.events[0] else { unreachable!() };
    assert_eq!(call.caller.as_str(), "principal-me");
    assert_eq!(call.started_at, Timestamp(1_790_000_000_000));
    assert!(translator.call_row(&row("dm-ana", "outgoing_ringing", 1)).events.is_empty());
    let connecting = translator.call_row(&row("dm-ana", "connecting", 2));
    assert_eq!(calls(&connecting.events), [(CallState::Connecting, 2)]);
    // An older row arriving late changes nothing.
    assert!(translator.call_row(&row("dm-ana", "outgoing_ringing", 1)).events.is_empty());
    let mut ended = row("dm-ana", "ended", 3);
    ended["end_reason"] = json!("something-new");
    assert_eq!(
        calls(&translator.call_row(&ended).events),
        [(CallState::Ended(CallEndReason::Failed), 3)]
    );
}

#[test]
fn a_row_waits_for_its_conversation_to_be_listed() {
    let mut translator = translator();
    let early = translator.call_row(&row("dm-new", "incoming_ringing", 1));
    assert!(early.events.is_empty());
    assert_eq!(early.follow, [Fetch::first(Paged::Conversations)]);
    let mut listing = frames()["conversation.directory"].clone();
    let mut new = listing["conversations"][0].clone();
    new["id"] = json!("dm-new");
    listing["conversations"].as_array_mut().unwrap().push(new);
    let listed = translator.fetched(&Fetch::first(Paged::Conversations), &listing, Timestamp(0));
    assert_eq!(calls(&listed.events), [(CallState::IncomingRinging, 1)]);
}

#[test]
fn malformed_rows_are_dropped() {
    let mut translator = translator();
    for broken in [
        json!({ "call_id": "call-1" }),
        json!({ "state": "connecting" }),
        {
            let mut r = row("dm-ana", "dancing", 1);
            r["state"] = json!("dancing");
            r
        },
        {
            let mut r = row("dm-ana", "connecting", 1);
            r["direction"] = json!("sideways");
            r
        },
        {
            let mut r = row("dm-ana", "connecting", 1);
            r["seq"] = Value::Null;
            r
        },
    ] {
        let translated = translator.call_row(&broken);
        assert!(translated.events.is_empty() && translated.follow.is_empty(), "{broken}");
    }
}

#[test]
fn a_media_session_is_read_and_its_secrets_wiped() {
    let key = |byte: u8| ramflux_protocol::encode_base64url([byte; 32]);
    let answer = json!({
        "call_id": "call-1",
        "relay": { "endpoint": "relay.example:3478", "token": ramflux_protocol::encode_base64url(b"tok") },
        "sframe": {
            "cipher_suite": 5,
            "send": { "kid": 7, "base_key": key(1) },
            "receive": { "kid": 8, "base_key": key(2) },
        },
    });
    let media = call_media(&CallId::new("call-1"), answer.clone()).expect("session");
    assert_eq!(media.endpoint, "relay.example:3478");
    assert_eq!(&media.token[..], b"tok");
    assert_eq!((media.suite, media.send.kid(), media.receive.kid()), (5, 7, 8));
    assert_eq!(media.send.base_key(), [1; 32]);
    assert!(!format!("{media:?}").contains(&key(1)));
    for broken in [
        {
            let mut a = answer.clone();
            a["relay"]["endpoint"] = json!("");
            a
        },
        {
            let mut a = answer.clone();
            a["sframe"]["send"]["base_key"] = json!("!!");
            a
        },
        {
            let mut a = answer.clone();
            a["sframe"]["receive"]["kid"] = Value::Null;
            a
        },
        {
            let mut a = answer.clone();
            a["sframe"]["cipher_suite"] = json!(70_000);
            a
        },
    ] {
        assert_eq!(call_media(&CallId::new("call-1"), broken).unwrap_err(), "backend.call.media");
    }
}
