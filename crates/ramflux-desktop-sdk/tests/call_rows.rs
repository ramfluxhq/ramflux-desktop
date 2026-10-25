// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The translator's view of call rows and media sessions: each sequence
//! number is reported once, a row for a conversation not listed yet waits
//! for the listing it asks for, a malformed row is dropped, `call.list` and
//! the call records of `message.list` are the same rows, the daemon's end
//! reasons are read as such, and a media session is read with its secrets
//! wiped from the answer.

mod common;

use common::*;
use ramflux_desktop_core::{
    AccountId, CallEndReason, CallId, CallState, ContextId, DomainEvent, Timestamp,
};
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
        "media_generation": 0,
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
fn listed_calls_and_call_records_are_the_same_rows() {
    let mut translator = translator();
    let listed = translator.fetched(
        &Fetch::Calls,
        &json!({ "calls": [row("dm-ana", "incoming_ringing", 2)] }),
        Timestamp(0),
    );
    assert_eq!(calls(&listed.events), [(CallState::IncomingRinging, 2)]);
    let mut ended = row("dm-ana", "ended", 3);
    ended["end_reason"] = json!("missed");
    ended["ended_at"] = json!(1_790_000_045);
    let messages = json!({ "messages": [{
        "id": "call-1", "message_id": "call-1", "conversation_id": "dm-ana", "kind": "call",
        "created_at": 1_790_000_000, "call": ended, "duration_seconds": null, "missed": true,
    }] });
    let read =
        translator.fetched(&Fetch::Messages(ContextId::new("dm-ana")), &messages, Timestamp(0));
    assert_eq!(calls(&read.events), [(CallState::Ended(CallEndReason::Missed), 3)]);
    // Never a message moment of its own.
    assert!(!read.events.iter().any(|e| matches!(e, DomainEvent::MomentAdded(_))));
}

#[test]
fn the_daemons_end_reasons_are_read_as_such() {
    for (reason, read) in [
        ("completed", CallEndReason::Completed),
        ("cancelled", CallEndReason::Cancelled),
        ("declined", CallEndReason::Declined),
        ("busy", CallEndReason::Busy),
        ("timeout", CallEndReason::Timeout),
        ("answered_elsewhere", CallEndReason::AnsweredElsewhere),
        ("failed", CallEndReason::Failed),
        ("missed", CallEndReason::Missed),
    ] {
        let mut translator = translator();
        let mut ended = row("dm-ana", "ended", 4);
        ended["end_reason"] = json!(reason);
        assert_eq!(
            calls(&translator.call_row(&ended).events),
            [(CallState::Ended(read), 4)],
            "{reason}"
        );
    }
}

#[test]
fn a_media_session_is_read_and_its_secrets_wiped() {
    let key = |byte: u8| ramflux_protocol::encode_base64url([byte; 32]);
    let answer = json!({
        "relay": {
            "endpoint": "relay.example:3478",
            "token": key(9),
            "lease_id": ramflux_protocol::encode_base64url([4_u8; 16]),
            "side": 1,
            "rebind_key": key(3),
        },
        "sframe": {
            "cipher_suite": 5,
            "send": { "kid": 1, "base_key": key(1) },
            "receive": { "kid": 0, "base_key": key(2) },
        },
    });
    let media = call_media(&CallId::new("call-1"), answer.clone()).expect("session");
    assert_eq!(media.endpoint, "relay.example:3478");
    assert_eq!(&media.token[..], [9; 32]);
    assert_eq!((media.suite, media.send.kid(), media.receive.kid()), (5, 1, 0));
    assert_eq!(media.send.base_key(), [1; 32]);
    let rebind = media.rebind.as_ref().expect("rebind credential");
    assert_eq!((rebind.lease_id, rebind.side, &rebind.key[..]), ([4; 16], 1, &[3; 32][..]));
    let printed = format!("{media:?}");
    assert!(!printed.contains(&key(1)) && !printed.contains(&key(3)), "{printed}");
    let broken = |path: [&str; 2], value: Value| {
        let mut a = answer.clone();
        a[path[0]][path[1]] = value;
        a
    };
    for broken in [
        broken(["relay", "endpoint"], json!("")),
        broken(["relay", "lease_id"], json!(ramflux_protocol::encode_base64url([4_u8; 8]))),
        broken(["relay", "side"], json!(2)),
        broken(["relay", "rebind_key"], json!(ramflux_protocol::encode_base64url([3_u8; 16]))),
        broken(["relay", "rebind_key"], Value::Null),
        broken(["sframe", "cipher_suite"], json!(70_000)),
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
    ] {
        assert_eq!(call_media(&CallId::new("call-1"), broken).unwrap_err(), "backend.call.media");
    }
}
