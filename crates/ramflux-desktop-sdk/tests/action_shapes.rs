// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The actions the client directories unlock, as frames: each parses into
//! the product SDK's request type at the pinned revision (every field sent
//! is the type's, except an attachment's documented `name`), and its ids,
//! routes, devices and commitments are the ones the directories listed (the
//! fixture's answers, translated). Missing routes are refused with their
//! reason before anything is sent; nothing is guessed.

mod common;

use ramflux_desktop_core::{AccountId, ContextId, ContextKind, Effect, FailureCode, methods};
use ramflux_desktop_sdk::bus::BusRequest;
use ramflux_desktop_sdk::directory::Directory;
use ramflux_desktop_sdk::plan::{Known, Plan, plan};
use ramflux_sdk::{LocalBusGroupMemberAddRequest, LocalBusMessageSubmitRequest};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

struct Listed(Directory);

impl Known for Listed {
    fn context_kind(&self, context: &ContextId) -> Option<ContextKind> {
        match context.as_str() {
            "thread-team" => Some(ContextKind::Group),
            "dm-ana" | "dm-unrouted" => Some(ContextKind::Direct),
            _ => None,
        }
    }
    fn self_device(&self) -> Option<&str> {
        Some("device-me")
    }
    fn daemon_grant_id(&self, _: &str) -> Option<&str> {
        None
    }
    fn directory(&self) -> Option<&Directory> {
        Some(&self.0)
    }
}

fn planned_with(directory: Directory, method: &str, body: Value) -> Plan {
    let effect = Effect::Call { method: method.to_owned(), body };
    plan(Some(&AccountId::new("acct-main")), effect, &Listed(directory)).plan
}

fn planned(method: &str, body: Value) -> Plan {
    planned_with(common::directory(), method, body)
}

fn one(plan: Plan) -> BusRequest {
    match plan {
        Plan::Call { request, .. } => request,
        other => panic!("one request expected, got {other:?}"),
    }
}

fn refused(plan: Plan) -> String {
    match plan {
        Plan::Refuse { code: FailureCode::UnsupportedByBackend, message } => message,
        other => panic!("a refusal with a reason expected, got {other:?}"),
    }
}

/// The body parses into `T` and every field but `extras` survives.
fn typed<T: DeserializeOwned + serde::Serialize>(request: &BusRequest, extras: &[&str]) -> T {
    let body = request.wire_body();
    let parsed: T = serde_json::from_value(body.clone())
        .unwrap_or_else(|e| panic!("{}: not the SDK request type: {e}", request.method));
    let back = serde_json::to_value(&parsed).expect("json");
    for (field, value) in body.as_object().expect("object body") {
        if !extras.contains(&field.as_str()) {
            assert_eq!(
                back.get(field),
                Some(value),
                "{}: {field} is not the SDK's",
                request.method
            );
        }
    }
    parsed
}

#[test]
fn a_direct_message_is_submitted_over_the_listed_route() {
    let request =
        one(planned(methods::SEND_MESSAGE, json!({ "context_id": "dm-ana", "text": "hi" })));
    assert_eq!(request.method, "message.submit");
    assert!(request.operation_id.is_some(), "a send carries a retry key");
    let submit: LocalBusMessageSubmitRequest = typed(&request, &[]);
    assert_eq!(submit.conversation_id, "dm-ana");
    assert_eq!(submit.source_principal_id, "principal-me", "this account's principal");
    assert_eq!(submit.sender_id, "device-me");
    assert_eq!(submit.recipient_device_id.as_deref(), Some("device-ana"), "the route's device");
    assert_eq!(submit.recipient_principal_commitment.as_deref(), Some("cm-ana"));
    assert_eq!(submit.target_delivery_id, "target-ana");
    assert_eq!(submit.plaintext_body_base64, Some(ramflux_protocol::encode_base64url(b"hi")));
    assert!(submit.attachments.is_empty() && submit.federation.is_none());
    // One id: the recipient's daemon and the receipt name the message by
    // its envelope id.
    assert!(submit.message_id.starts_with("desk-msg-"));
    assert_eq!(submit.envelope_id, submit.message_id);
    assert_eq!(submit.ttl, 300);
}

#[test]
fn a_conversation_without_a_route_is_refused_with_the_reason() {
    let reason = refused(planned(
        methods::SEND_MESSAGE,
        json!({ "context_id": "dm-unrouted", "text": "x" }),
    ));
    assert_eq!(reason, "backend.gap.no_route");
    // No captured route (a conversation the contact started, or a selected
    // device no longer verified): the client selects the other
    // participant's verified device from the identity directory.
    let mut directory = common::directory();
    if let Some(dm) = directory.conversations.get_mut(&ContextId::new("dm-ana")) {
        dm.direct_route = None;
        dm.participants = vec!["principal-me".to_owned(), "principal-ana".to_owned()];
    }
    let send = json!({ "context_id": "dm-ana", "text": "x" });
    let request = one(planned_with(directory.clone(), methods::SEND_MESSAGE, send.clone()));
    let submit: LocalBusMessageSubmitRequest = typed(&request, &[]);
    assert_eq!(submit.recipient_device_id.as_deref(), Some("device-ana"));
    assert_eq!(submit.recipient_principal_commitment.as_deref(), Some("cm-ana"));
    assert_eq!(submit.target_delivery_id, "target-ana");
    // Without a verified device of the peer there is still no route.
    if let Some(ana) = directory.identities.get_mut("principal-ana") {
        ana.devices.clear();
    }
    assert_eq!(
        refused(planned_with(directory, methods::SEND_MESSAGE, send)),
        "backend.gap.no_route"
    );
}

#[test]
fn the_identity_calls_name_the_contact_commitment_and_the_shown_anchor() {
    let body = json!({ "participant_id": "principal-ana", "new_anchor": "A2" });
    let accept = planned(methods::IDENTITY_ACCEPT_CHANGE, body);
    let Plan::Call { request, refresh } = accept else { panic!("one call") };
    assert_eq!(request.method, "contact.trust.accept_change");
    assert_eq!(request.body, json!({ "contact": "cm-ana", "new_anchor": "A2" }));
    assert_eq!(refresh.len(), 1, "the status is read again");
    let status =
        one(planned(methods::IDENTITY_STATUS, json!({ "participant_id": "principal-ana" })));
    assert_eq!(status.method, "contact.verification.status");
    assert_eq!(status.body, json!({ "contact": "cm-ana" }));
    let security = one(planned(methods::CONVERSATION_SECURITY, json!({ "context_id": "dm-ana" })));
    assert_eq!(security.method, "conversation.security");
    assert_eq!(security.body, json!({ "conversation_id": "dm-ana" }));
}

#[test]
fn members_are_added_and_invited_per_verified_device() {
    let body =
        json!({ "context_id": "thread-team", "participant_id": "principal-cy", "role": "Member" });
    let Plan::Calls { requests, .. } = planned(methods::GROUP_ADD_MEMBER, body.clone()) else {
        panic!("one add per device of a two-device contact");
    };
    let devices: Vec<String> = requests
        .iter()
        .map(|request| {
            assert!(request.operation_id.is_some());
            let add: LocalBusGroupMemberAddRequest = typed(request, &[]);
            assert_eq!(add.group_id, "g-team");
            assert_eq!(add.member_principal_commitment.as_deref(), Some("cm-cy"));
            assert_eq!(add.role, "member");
            assert!(add.member_signing_public_key.is_some() && add.target_delivery_id.is_some());
            add.member_id
        })
        .collect();
    assert_eq!(devices, ["device-cy-1", "device-cy-2"]);
    let unknown = refused(planned(
        methods::GROUP_ADD_MEMBER,
        json!({ "context_id": "thread-team", "participant_id": "principal-zed", "role": "Member" }),
    ));
    assert_eq!(unknown, "backend.gap.no_member_route");
    let unlinked = refused(planned(
        methods::GROUP_ADD_MEMBER,
        json!({ "context_id": "dm-ana", "participant_id": "principal-cy", "role": "Member" }),
    ));
    assert_eq!(unlinked, "backend.gap.no_group_link");
}
