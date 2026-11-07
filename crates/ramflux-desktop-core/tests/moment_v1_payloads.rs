// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Serde contract of the message schema v1 payload fields (S10c c02a).

use ramflux_desktop_core::*;
use serde_json::json;

fn full_message() -> MessagePayload {
    MessagePayload {
        text: "A\u{1F600}\u{e9}".to_owned(),
        receipt: Receipt::Delivered,
        tombstone: false,
        attachment: None,
        franking: None,
        reply_to: Some(ReplyRef { message_id: MomentId::new("m-1") }),
        forward: Some(ForwardRef {
            from_context: Some(ContextId::new("c-1")),
            from_message: Some(MomentId::new("m-0")),
            from_name: Some("Kai".to_owned()),
        }),
        revision: 3,
        reactions: vec![ReactionCount { token: "\u{1F44D}".to_owned(), count: 2, mine: true }],
        spans: vec![
            TextSpan { offset: 1, length: 1, kind: SpanKind::Bold },
            TextSpan {
                offset: 0,
                length: 3,
                kind: SpanKind::Mention { participant: ParticipantId::new("p-kai") },
            },
            TextSpan {
                offset: 2,
                length: 1,
                kind: SpanKind::Link { url: "https://example.com".to_owned() },
            },
        ],
        media: vec![MediaView {
            object_id: ObjectId::new("o-1"),
            name: "cat.png".to_owned(),
            mime: "image/png".to_owned(),
            size: 2048,
            state: MediaState::Revoked,
            thumbnail: Some(Thumbnail { width: 4, height: 2, bytes: vec![1, 2, 3] }),
        }],
        delete_scope: Some(DeleteScope::Everyone),
    }
}

#[test]
fn moment_v1_payloads_roundtrip() {
    let message = MomentKind::Message(full_message());
    let text = serde_json::to_string(&message).expect("serialize message");
    let back: MomentKind = serde_json::from_str(&text).expect("decode message");
    assert_eq!(back, message, "full v1 message round-trips");

    let fallback = MomentKind::Fallback(FallbackPayload {
        reason: "unsupported".to_owned(),
        text: "newer message type".to_owned(),
        unsupported: Some(UnsupportedInfo {
            kind: "poll".to_owned(),
            layer: "content".to_owned(),
            reason: "unknown_kind".to_owned(),
            raw_length: 412,
        }),
    });
    let text = serde_json::to_string(&fallback).expect("serialize fallback");
    let back: MomentKind = serde_json::from_str(&text).expect("decode fallback");
    assert_eq!(back, fallback, "fallback with unsupported info round-trips");
}

#[test]
fn moment_v1_payloads_roundtrip_r1_shaped_json_decodes() {
    let r1 = json!({"Message": {
        "text": "hi", "receipt": "Sent", "tombstone": false,
        "attachment": null, "franking": null
    }});
    let MomentKind::Message(payload) = serde_json::from_value(r1).expect("r1-shaped JSON decodes")
    else {
        panic!("expected a message");
    };
    assert_eq!(payload.text, "hi");
    assert!(payload.reactions.is_empty(), "reactions default to empty");
    assert!(payload.spans.is_empty() && payload.media.is_empty());
    assert_eq!((payload.revision, payload.reply_to, payload.delete_scope), (0, None, None));

    let r1_fallback = json!({"Fallback": {"reason": "r", "text": "t"}});
    let MomentKind::Fallback(fallback) =
        serde_json::from_value(r1_fallback).expect("r1-shaped fallback decodes")
    else {
        panic!("expected a fallback");
    };
    assert_eq!(fallback.unsupported, None);
}
