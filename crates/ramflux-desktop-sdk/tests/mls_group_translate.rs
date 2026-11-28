// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The MLS group replies and frames of method table r4 parse, and every
//! refusal code maps to a text.

use ramflux_desktop_core::mls_group::*;
use ramflux_desktop_core::{DomainEvent, Timestamp, methods};
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
    for kind in ["poll", "banana"] {
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

fn refused(code: &str, details: Value) -> BusError {
    BusError::Daemon {
        retry_after_ms: None,
        code: code.to_owned(),
        message: "x".to_owned(),
        reason_code: None,
        details: Some(Box::new(details)),
        cause: None,
    }
}

#[test]
fn the_m3_group_events_parse_with_their_body_nesting() {
    let pending = group_event(&json!({ "seq": 10, "group_id": "g", "kind": "pending_join",
        "body": { "request_id": "r-1", "anchor": "anc", "display": "Fay", "ai_flag": true,
                  "requested_at": 5, "expires_at": 9 } }))
    .expect("a pending join");
    assert!(matches!(
        pending,
        DomainEvent::MlsGroupEvent { kind: GroupEventKind::PendingJoin(ref r), .. } if r.ai_flag
    ));
    let catch_up = group_event(&json!({ "seq": 11, "group_id": "g", "kind": "catch_up",
        "body": { "state": "running", "done": 4, "total": 120 } }))
    .expect("a catch-up");
    assert!(matches!(
        catch_up,
        DomainEvent::MlsGroupEvent {
            kind: GroupEventKind::CatchUp { state: CatchUpState::Running, done: 4, total: 120 },
            ..
        }
    ));
    let unknown_state = group_event(&json!({ "seq": 12, "group_id": "g", "kind": "catch_up",
        "body": { "state": "weird" } }))
    .expect("an unknown state is idle");
    assert!(matches!(
        unknown_state,
        DomainEvent::MlsGroupEvent {
            kind: GroupEventKind::CatchUp { state: CatchUpState::Idle, .. },
            ..
        }
    ));
    for (code, params, expected) in [
        ("fork_alarm", json!({ "head_n": 2 }), "fork_alarm"),
        ("rejoin_needed", json!({ "reason": "state_lost" }), "rejoin_needed"),
        ("undecryptable", json!({ "count": 12, "from_log_seq": 140 }), "undecryptable"),
        ("admission_needs_online", json!({}), "admission_needs_online"),
        ("host_unusual", json!({ "kind": "late_roll" }), "host_unusual"),
        ("head_commit_rejected", json!({ "log_seq": 250 }), "head_commit_rejected"),
        ("weird", json!({}), "weird"),
    ] {
        let event = group_event(&json!({ "seq": 13, "group_id": "g", "kind": "warning",
            "body": { "code": code, "params": params, "cleared": true } }))
        .expect("a warning");
        let DomainEvent::MlsGroupEvent { kind: GroupEventKind::Warning { code, cleared }, .. } =
            event
        else {
            panic!("a warning event");
        };
        assert!(cleared);
        assert_eq!(code.code(), expected);
    }
    // A void arrives as its own kind and as a post: one row.
    let as_kind = group_event(&json!({ "seq": 14, "group_id": "g", "kind": "void", "log_seq": 251,
        "body": { "voided_log_seq": 250, "by": author() } }))
    .expect("a void");
    let as_post = group_event(&json!({ "seq": 15, "group_id": "g", "kind": "post", "log_seq": 251,
        "post": { "log_seq": 251, "post_id": "p-251", "kind": "void", "state": "sent",
                  "body": { "voided_log_seq": 250, "by": author() } } }))
    .expect("a void post");
    for event in [as_kind, as_post] {
        let DomainEvent::MlsGroupEvent { kind: GroupEventKind::Void(row), .. } = event else {
            panic!("a void event");
        };
        assert_eq!(row.log_seq, Some(251));
        assert!(matches!(row.body, PostBody::Void { voided_log_seq: Some(250), .. }));
    }
}

#[test]
fn the_m3_replies_become_their_outcomes_with_the_ids_of_the_call() {
    let admin = json!({ "group_id": "g", "anchor": "anc-ben", "role": "admin", "epoch": 3, "action_id": 9 });
    let ok = outcome(
        methods::MLS_GROUP_ROLE_SET,
        &admin,
        &Ok(json!({ "state": "committed", "log_seq": 310 })),
    );
    assert!(matches!(
        ok.as_slice(),
        [DomainEvent::MlsAdminOutcome {
            epoch: 3,
            action_id: 9,
            kind: AdminKind::Role,
            result: AdminResult::Committed { log_seq: 310 },
            ..
        }]
    ));
    // A reply with no position cannot be confirmed by an event.
    let bare = outcome(methods::MLS_GROUP_ROLE_SET, &admin, &Ok(json!({ "state": "committed" })));
    assert!(matches!(
        bare.as_slice(),
        [DomainEvent::MlsAdminOutcome { result: AdminResult::Refused { .. }, .. }]
    ));
    let owner = outcome(
        methods::MLS_GROUP_ROLE_SET,
        &admin,
        &Err(refused("not_admin", json!({ "required": "owner" }))),
    );
    assert!(matches!(
        owner.as_slice(),
        [DomainEvent::MlsAdminOutcome { result: AdminResult::Refused { code, .. }, .. }] if code == "not_owner"
    ));
    let paced = outcome(
        methods::MLS_GROUP_MEMBER_REMOVE,
        &json!({ "group_id": "g", "anchor": "a", "epoch": 1, "action_id": 2 }),
        &Err(refused("paced", json!({ "retry_at": 5_000 }))),
    );
    assert!(matches!(
        paced.as_slice(),
        [DomainEvent::MlsAdminOutcome {
            result: AdminResult::Refused { retry_at: Some(Timestamp(5_000)), .. },
            ..
        }]
    ));
    let void = outcome(
        methods::MLS_GROUP_VOID,
        &json!({ "group_id": "g", "log_seq": 250, "epoch": 1, "action_id": 3 }),
        &Err(refused("void_not_head", json!({ "head_log_seq": 252 }))),
    );
    assert!(matches!(
        void.as_slice(),
        [DomainEvent::MlsAdminOutcome {
            result: AdminResult::Refused { head_log_seq: Some(252), .. },
            ..
        }]
    ));
    let unknown = outcome(
        methods::MLS_GROUP_VOID,
        &json!({ "group_id": "g", "log_seq": 250, "epoch": 1, "action_id": 3 }),
        &Err(BusError::daemon("limit", "x")),
    );
    assert!(matches!(
        unknown.as_slice(),
        [DomainEvent::MlsAdminOutcome { result: AdminResult::Refused { code, .. }, .. }] if code == "generic"
    ));
    let not_found = outcome(
        methods::MLS_GROUP_REQUESTS_APPROVE,
        &json!({ "group_id": "g", "request_id": "r", "epoch": 1, "action_id": 4 }),
        &Err(BusError::daemon("request_not_found", "x")),
    );
    assert!(matches!(
        not_found.as_slice(),
        [DomainEvent::MlsAdminOutcome { result: AdminResult::Refused { code, .. }, .. }] if code == "request_not_found"
    ));
    let decline = outcome(
        methods::MLS_GROUP_REQUESTS_DECLINE,
        &json!({ "group_id": "g", "request_id": "r", "epoch": 2 }),
        &Ok(json!({ "state": "declined" })),
    );
    assert!(matches!(
        decline.as_slice(),
        [DomainEvent::MlsDeclineOutcome { epoch: 2, result: DeclineResult::Declined, .. }]
    ));
    let push = outcome(
        methods::MLS_GROUP_PUSH_SET,
        &json!({ "group_id": "g", "push": "none", "epoch": 2 }),
        &Ok(json!({ "push": "none" })),
    );
    assert!(matches!(
        push.as_slice(),
        [DomainEvent::MlsPushOutcome { result: PushResult::Set(PushSetting::None), .. }]
    ));
    let skip = outcome(
        methods::MLS_GROUP_CATCH_UP,
        &json!({ "group_id": "g", "skip": true, "epoch": 2, "from_seq": 5 }),
        &Ok(json!({ "state": "running", "done": 1, "total": 9 })),
    );
    assert!(matches!(
        skip.as_slice(),
        [DomainEvent::MlsSkipOutcome {
            from_seq: 5,
            result: SkipResult::Running { done: 1, total: 9 },
            ..
        }]
    ));
}

#[test]
fn a_requests_page_keeps_its_generation_and_a_failed_page_its_code() {
    let body = json!({ "group_id": "g", "cursor": null, "limit": 200, "page_gen": 4 });
    let page = outcome(
        methods::MLS_GROUP_REQUESTS_LIST,
        &body,
        &Ok(json!({ "requests": [
            { "request_id": "r-1", "anchor": "a", "display": null, "ai_flag": false, "requested_at": 1, "expires_at": 2 },
            { "no_request_id": true }
        ], "cursor": "c2" })),
    );
    assert!(matches!(
        page.as_slice(),
        [DomainEvent::MlsRequestsPage { page_gen: 4, first_page: true, rows, cursor: Some(_), failed: None, .. }] if rows.len() == 1
    ));
    let failed = outcome(
        methods::MLS_GROUP_REQUESTS_LIST,
        &json!({ "group_id": "g", "cursor": "c2", "page_gen": 4 }),
        &Err(BusError::daemon("host_unreachable", "x")),
    );
    assert!(matches!(
        failed.as_slice(),
        [DomainEvent::MlsRequestsPage { first_page: false, failed: Some(code), .. }] if code == "host_unreachable"
    ));
}

#[test]
fn the_not_owner_code_has_its_key() {
    assert_eq!(error_key("not_owner"), "group.error.not_owner");
}
