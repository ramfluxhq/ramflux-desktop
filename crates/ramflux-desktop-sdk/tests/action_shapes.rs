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
use ramflux_desktop_sdk::bus::{BusRequest, wipe};
use ramflux_desktop_sdk::directory::Directory;
use ramflux_desktop_sdk::plan::{Known, Plan, plan};
use ramflux_sdk::{
    LocalBusContactSafetyRequest, LocalBusGroupInviteCreateRequest, LocalBusGroupMemberAddRequest,
    LocalBusMessageSubmitRequest, LocalBusObjectGetRequest, LocalBusObjectTransferResumeRequest,
};
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
        Plan::Call { request, .. } | Plan::Open { request, .. } => request,
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
    assert!(
        submit.message_id.starts_with("desk-msg-") && submit.envelope_id.starts_with("desk-env-")
    );
    assert_eq!(submit.ttl, 300);
}

#[test]
fn a_conversation_without_a_route_is_refused_with_the_reason() {
    let reason = refused(planned(
        methods::SEND_MESSAGE,
        json!({ "context_id": "dm-unrouted", "text": "x" }),
    ));
    assert_eq!(reason, "backend.gap.no_route");
    // A route whose selected device is no longer verified is listed as null.
    let mut directory = common::directory();
    if let Some(dm) = directory.conversations.get_mut(&ContextId::new("dm-ana")) {
        dm.direct_route = None;
    }
    let plan = planned_with(
        directory,
        methods::SEND_MESSAGE,
        json!({ "context_id": "dm-ana", "text": "x" }),
    );
    assert_eq!(refused(plan), "backend.gap.no_route");
}

#[test]
fn an_attachment_rides_on_the_direct_message_through_the_node_relay() {
    let dir = std::env::temp_dir().join(format!("rf-attach-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let file = dir.join("notes.txt");
    std::fs::write(&file, b"attached bytes").expect("file");
    let body = json!({ "context_id": "dm-ana", "path": file.to_string_lossy() });
    let request = one(planned(methods::ATTACH_OBJECT, body));
    let submit: LocalBusMessageSubmitRequest = typed(&request, &["attachments"]);
    let attachment = &submit.attachments[0];
    assert_eq!(attachment.object_id, format!("attachment:{}:0", submit.message_id));
    assert_eq!(attachment.plaintext_base64, ramflux_protocol::encode_base64url(b"attached bytes"));
    assert_eq!(attachment.relay_endpoint, "quic://harbor.example.org:7543", "the node's relay");
    assert_eq!(attachment.relay_service_key_base64, None, "no service key is sent");
    let wire = request.wire_body();
    let sent = wire["attachments"][0].as_object().expect("attachment").clone();
    let back = serde_json::to_value(attachment).expect("json");
    for (field, value) in &sent {
        if field == "name" {
            assert_eq!(value, "notes.txt", "the documented file name");
        } else {
            assert_eq!(back.get(field), Some(value), "attachment {field} is not the SDK's");
        }
    }
    wipe(wire);
    // Over the one-frame limit: refused before reading it into a frame.
    let big = dir.join("big.bin");
    let limit = ramflux_sdk::MAX_LOCAL_BUS_ONE_SHOT_OBJECT_BYTES;
    std::fs::write(&big, vec![0_u8; limit + 1]).expect("big file");
    let reason = refused(planned(
        methods::ATTACH_OBJECT,
        json!({ "context_id": "dm-ana", "path": big.to_string_lossy() }),
    ));
    assert_eq!(reason, format!(r#"backend.failure.attachment_too_large {{"limit":"{limit}"}}"#));
    let group = refused(planned(
        methods::ATTACH_OBJECT,
        json!({ "context_id": "thread-team", "path": file.to_string_lossy() }),
    ));
    assert_eq!(group, "backend.gap.group_attachments");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn verification_names_the_contact_commitment_from_the_identity_directory() {
    let request =
        one(planned(methods::SAFETY_VERIFY, json!({ "participant_id": "principal-ana" })));
    assert_eq!(request.method, "contact.verify");
    let verify: LocalBusContactSafetyRequest = typed(&request, &[]);
    assert_eq!(verify.contact_identity_commitment, "cm-ana");
    let unknown =
        refused(planned(methods::SAFETY_VERIFY, json!({ "participant_id": "principal-zed" })));
    assert_eq!(unknown, "backend.gap.no_commitment");
}

#[test]
fn objects_are_opened_saved_and_resumed_by_their_directory_ids() {
    let pull = planned(methods::OBJECT_PULL, json!({ "object_id": "attachment:m1:0" }));
    let Plan::Open { request, save_to: None } = pull else { panic!("an object read") };
    assert!(request.operation_id.is_none(), "an object read keeps no receipt of the bytes");
    let get: LocalBusObjectGetRequest = typed(&request, &[]);
    assert_eq!(get.object_id, "attachment:m1:0");
    assert!(
        get.relay_endpoint.is_none() && !get.relay_ack,
        "no endpoint: the daemon picks the node relay or the stored copy"
    );
    let save =
        planned(methods::OBJECT_SAVE, json!({ "object_id": "attachment:m1:0", "path": "/d/n" }));
    assert!(matches!(save, Plan::Open { save_to: Some(ref path), .. } if path == "/d/n"));
    let resume = one(planned(methods::OBJECT_RETRY, json!({ "object_id": "attachment:m2:0" })));
    assert!(resume.operation_id.is_some());
    let resumed: LocalBusObjectTransferResumeRequest = typed(&resume, &[]);
    assert_eq!(resumed.direction, "upload", "an outgoing object resumes its upload");
    assert_eq!(resumed.relay_endpoint.as_deref(), Some("quic://harbor.example.org:7543"));
    let incoming = one(planned(methods::OBJECT_RETRY, json!({ "object_id": "attachment:m1:0" })));
    assert_eq!(incoming.wire_body()["direction"], "download");
    let unknown = refused(planned(methods::OBJECT_RETRY, json!({ "object_id": "legacy-object" })));
    assert_eq!(unknown, "backend.gap.no_direction");
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
    let invite = one(planned(
        methods::GROUP_CREATE_INVITE,
        json!({ "context_id": "thread-team", "participant_id": "principal-ana", "role": "Member" }),
    ));
    let created: LocalBusGroupInviteCreateRequest = typed(&invite, &[]);
    assert_eq!(
        (
            created.invitee_id.as_str(),
            created.actor_id.as_str(),
            created.target_delivery_id.as_str()
        ),
        ("device-ana", "device-me", "target-ana")
    );
    assert_eq!(created.invitee_signing_public_key, "key-ana");
    assert_eq!(created.invitee_principal_commitment.as_deref(), Some("cm-ana"));
    assert!(created.expires_at > 1_700_000_000);
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
