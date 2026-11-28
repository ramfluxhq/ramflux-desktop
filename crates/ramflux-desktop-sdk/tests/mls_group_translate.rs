// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The MLS group replies and frames of method table r4 parse, and every
//! refusal code maps to a text.

use ramflux_desktop_core::mls_group::*;
use ramflux_desktop_core::{DomainEvent, methods};
use ramflux_desktop_sdk::bus::BusError;
use ramflux_desktop_sdk::translate::mls_group::{
    error_code, group_event, join_outcome, outcome, post, send_state, summary,
};
use serde_json::{Value, json};

fn author() -> Value {
    json!({ "anchor": "a1", "display": null, "ai_flag": true, "is_me": false })
}

#[test]
fn every_r4_sample_shape_parses() {
    let row = json!({
        "group_id": "g", "name": "Design", "tier": 1000, "members": 12, "leaves": 0,
        "agents": 1, "approval": true, "my_role": "owner", "host_node": "node.example",
        "push": "all", "catch_up": { "state": "running", "done": 1, "total": 2 },
        "privacy_delay": { "send_after": null }, "quota": { "post": { "limit": 20, "remaining": 3 } },
        "last_log_seq": 9, "warnings": [{ "code": "host_ip_visible", "params": {} }],
        "last_activity_at": 5, "unread": 2, "unread_mentions": 1, "read_log_seq": 7,
        "last_post": { "log_seq": 9, "kind": "text", "author": author(), "snippet": "hi", "appended_at": 4 }
    });
    let parsed = summary(&row).expect("a group");
    assert_eq!(parsed.privacy_delay, PrivacyDelay::Armed);
    assert!(parsed.host_ip_visible() && parsed.unread == 2 && parsed.last_post.is_some());

    let text = post(&json!({ "log_seq": 3, "post_id": "p", "author": author(), "kind": "text",
        "body": "hello", "mentions": [], "state": "sent" }))
    .expect("a post");
    assert_eq!(text.body, PostBody::Text("hello".into()));
    let membership = post(&json!({ "log_seq": 4, "post_id": "p2", "kind": "membership", "state": "sent",
        "body": { "changes": [{ "change": "joined", "anchor": "x", "display": null, "ai_flag": false, "is_me": true }],
                  "by": author() } }))
    .expect("a membership row");
    assert!(
        matches!(membership.body, PostBody::Membership { ref changes, .. } if changes[0].is_me)
    );
    let settings =
        post(&json!({ "log_seq": 5, "post_id": "p3", "kind": "settings", "state": "sent",
        "body": { "name": "X", "approval": false, "by": author() } }))
        .expect("a settings row");
    assert!(matches!(settings.body, PostBody::Settings { .. }));
    let pending = post(&json!({ "post_id": "p4", "kind": "text", "body": "x", "state": "pending",
        "send": { "state": "delayed_privacy", "params": { "send_after": 9 } } }))
    .expect("an unsent own row");
    assert_eq!(pending.log_seq, None);
    assert!(matches!(pending.send, Some(SendState::Delayed { .. })));
    // An unknown kind and an unknown state become the Unknown cases.
    let odd = post(&json!({ "log_seq": 6, "post_id": "p5", "kind": "poll", "state": "weird" }))
        .expect("an unknown row");
    assert!(matches!(odd.kind, PostKind::Unknown(_)) && matches!(odd.state, PostState::Unknown(_)));

    assert!(matches!(
        send_state(
            "rate_limited",
            &json!({ "limit": 20, "remaining": 0, "first_day": true, "resets_at": 7 }),
            None
        ),
        SendState::NotSent(NotSentReason::RateLimited { remaining: 0, .. })
    ));
    assert!(matches!(
        send_state("parked", &json!({}), None),
        SendState::Queued(QueueReason::Unknown)
    ));
    assert_eq!(
        join_outcome(
            &json!({ "state": "refused", "reason": "already_member", "params": { "group_id": "g" } })
        ),
        JoinOutcome::Refused(JoinRefusal::AlreadyMember { group_id: Some("g".into()) })
    );
}

#[test]
fn group_events_parse_and_other_kinds_are_skipped() {
    let post_state = group_event(&json!({ "seq": 1, "group_id": "g", "kind": "post_state",
        "post_id": "p", "state": "sent", "params": { "log_seq": 8 }, "log_seq": 8 }))
    .expect("a post state");
    assert!(matches!(
        post_state,
        DomainEvent::MlsGroupEvent {
            kind: GroupEventKind::PostState { state: SendState::Sent { log_seq: 8 }, .. },
            ..
        }
    ));
    let deleted = group_event(&json!({ "seq": 2, "group_id": "g", "kind": "post_deleted",
        "post_id": "p", "post_log_seq": 3, "log_seq": 9 }))
    .expect("a deletion");
    assert!(matches!(
        deleted,
        DomainEvent::MlsGroupEvent {
            kind: GroupEventKind::PostDeleted { post_log_seq: Some(3), .. },
            ..
        }
    ));
    let joined = group_event(&json!({ "seq": 3, "group_id": "g", "kind": "join_result",
        "request_id": "r", "state": "refused", "reason": "request_expired", "params": { "expired_at": 5 } }))
    .expect("a join result");
    assert!(matches!(
        joined,
        DomainEvent::MlsGroupEvent {
            kind: GroupEventKind::JoinResult {
                result: Err(JoinRefusal::RequestExpired { .. }),
                ..
            },
            ..
        }
    ));
    for kind in ["pending_join", "catch_up", "warning", "void"] {
        assert!(group_event(&json!({ "seq": 4, "group_id": "g", "kind": kind })).is_none());
    }
}

#[test]
fn every_refusal_code_maps_to_its_key() {
    for code in ERROR_CODES {
        assert_eq!(error_key(code), format!("group.error.{code}"));
    }
    assert_eq!(error_key("void_not_head"), "group.error.void_not_head");
    assert_eq!(error_key("unavailable"), "group.error.unavailable");
    assert_eq!(error_key("something_new"), "group.error.generic");
    let cases = [
        ("link_invalid", "group.join.refused.link_invalid"),
        ("invite_expired", "group.join.refused.invite_expired"),
        ("invite_invalid", "group.join.refused.invite_invalid"),
        ("group_full", "group.join.refused.group_full"),
        ("queue_full", "group.join.refused.queue_full"),
        ("request_expired", "group.join.refused.request_expired"),
        ("already_member", "group.join.refused.already_member"),
        ("rate_limited", "group.join.refused.rate_limited"),
        ("invite_unknown", "group.join.refused.generic"),
        ("banana", "group.join.refused.generic"),
    ];
    for (reason, key) in cases {
        assert_eq!(
            ramflux_desktop_sdk::translate::mls_group::join_refusal(reason, &Value::Null).key(),
            key
        );
    }
}

#[test]
fn error_frames_become_the_typed_outcomes() {
    let error = BusError::daemon("host_unreachable", "x");
    assert_eq!(error_code(&error), "host_unreachable");
    let join = outcome(methods::MLS_GROUP_JOIN, &json!({ "attempt": 3 }), &Err(error));
    assert!(matches!(
        join.as_slice(),
        [DomainEvent::MlsJoinOutcome { attempt: 3, outcome: JoinOutcome::Failed { .. } }]
    ));
    let owner = BusError::daemon("owner_protected", "x");
    let leave = outcome(methods::MLS_GROUP_LEAVE, &json!({ "group_id": "g" }), &Err(owner));
    assert!(matches!(
        leave.as_slice(),
        [DomainEvent::MlsLeft { result: LeaveResult::OwnerProtected, .. }]
    ));
}
