// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Every account, node, grant, conversation, device, group, message, object
//! and pull frame the desktop builds parses into the product SDK's request
//! type at the pinned revision, and every field it sends is a field of that
//! type (serde would drop an unknown one silently), except the client
//! metadata fields the read model documents (`account.create` `label` /
//! `kind`, an attachment's `name`), which the daemon reads from the raw body.
//! Contact frames are checked the same way in `contacts.rs`. Mutating effects
//! carry a fresh retry key; reads, pulls and object reads carry none. The
//! routes, devices and commitments come from the directories (`Fake`).

use ramflux_desktop_core::{AccountId, ContextId, ContextKind, Effect, SecretInput, methods};
use ramflux_desktop_sdk::bus::BusRequest;
use ramflux_desktop_sdk::directory::Directory;
use ramflux_desktop_sdk::plan::fetch::{Fetch, Paged};
use ramflux_desktop_sdk::plan::{Known, Plan, plan};
use ramflux_sdk::{
    LocalBusAccountBackupExportRequest, LocalBusAccountBackupImportRequest,
    LocalBusAccountCreateRequest, LocalBusAccountPassphraseRotateRequest,
    LocalBusAccountUnlockRequest, LocalBusConversationDisappearingSetRequest,
    LocalBusConversationMuteRequest, LocalBusConversationRequest, LocalBusDeviceRevokeRequest,
    LocalBusGrantRevokeRequest, LocalBusGrantStandingApprovalRevokeRequest,
    LocalBusGroupMemberBanRequest, LocalBusGroupMemberKickRequest,
    LocalBusGroupMemberRemoveRequest, LocalBusGroupReceiveRequest, LocalBusGroupRoleSetRequest,
    LocalBusGroupSendRequest, LocalBusMcpApprovalDecisionRequest, LocalBusMessageReceiveRequest,
    NodePinSource,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

mod common;

struct Fake(Directory);

impl Known for Fake {
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
        Some("grant-1")
    }
    fn directory(&self) -> Option<&Directory> {
        Some(&self.0)
    }
}

fn call(method: &str, body: Value) -> Effect {
    Effect::Call { method: method.to_owned(), body }
}

fn secret(method: &str, body: Value, secrets: &[&str]) -> Effect {
    Effect::SecretCall {
        method: method.to_owned(),
        body,
        secrets: secrets.iter().map(|s| SecretInput::new(*s)).collect(),
    }
}

fn fake() -> Fake {
    Fake(common::directory())
}

fn planned(effect: Effect) -> Plan {
    plan(Some(&AccountId::new("acct-main")), effect, &fake()).plan
}

fn request(effect: Effect) -> BusRequest {
    match planned(effect) {
        Plan::Call { request, .. } | Plan::Open { request, .. } => request,
        other => panic!("expected a call, got {other:?}"),
    }
}

/// The body parses into `T` and every field sent survives the round trip.
fn parses<T: DeserializeOwned + Serialize>(request: &BusRequest) -> T {
    parses_with(request, &[])
}

/// As [`parses`], allowing the documented client metadata `extras`.
fn parses_with<T: DeserializeOwned + Serialize>(request: &BusRequest, extras: &[&str]) -> T {
    let body = request.wire_body();
    let typed: T = serde_json::from_value(body.clone())
        .unwrap_or_else(|e| panic!("{}: not the SDK request type: {e}", request.method));
    let back = serde_json::to_value(&typed).expect("json");
    for (field, value) in body.as_object().expect("an object body") {
        if extras.contains(&field.as_str()) {
            continue;
        }
        assert_eq!(
            back.get(field),
            Some(value),
            "{}: field {field} is not the SDK's",
            request.method
        );
    }
    typed
}

/// A mutating request: carries a retry key.
fn keyed(effect: Effect) -> BusRequest {
    let request = request(effect);
    let key = request.operation_id.as_deref().unwrap_or_default();
    assert!(!key.is_empty() && key.len() <= 256, "{} carries a retry key", request.method);
    request
}

/// A mutating request: parses into `T` and carries a retry key.
fn mutation<T: DeserializeOwned + Serialize>(effect: Effect) -> T {
    parses(&keyed(effect))
}

#[test]
fn account_frames_are_the_sdk_account_requests() {
    let created = keyed(secret(
        methods::ACCOUNT_CREATE,
        json!({ "kind": "HumanInteractive", "label": "Me", "node_id": "n-harbor" }),
        &["pw"],
    ));
    let create: LocalBusAccountCreateRequest = parses_with(&created, &["label", "kind"]);
    let wire = created.wire_body();
    assert_eq!((wire["label"].as_str(), wire["kind"].as_str()), (Some("Me"), Some("human")));
    ramflux_desktop_sdk::bus::wipe(wire);
    assert_eq!(create.node_id.as_deref(), Some("n-harbor"));
    assert_eq!(create.account_secret, "pw");
    assert_eq!(create.client_mode, ramflux_sdk::LocalBusClientMode::AttendedCli);
    assert_ne!(create.root_seed, create.device_seed);
    let unlock: LocalBusAccountUnlockRequest =
        mutation(secret(methods::ACCOUNT_UNLOCK, json!({ "account_id": "acct-x" }), &["pw"]));
    assert_eq!(unlock.passphrase, "pw");
    let import: LocalBusAccountBackupImportRequest =
        mutation(secret(methods::ACCOUNT_BACKUP_IMPORT, json!({ "path": "/b" }), &["pw"]));
    assert_eq!(import.input_path, "/b");
    let export: LocalBusAccountBackupExportRequest = mutation(secret(
        methods::ACCOUNT_BACKUP_EXPORT,
        json!({ "account_id": "acct-main", "path": "/o" }),
        &["pw"],
    ));
    assert_eq!(export.output_path, "/o");
    let rotate: LocalBusAccountPassphraseRotateRequest = mutation(secret(
        methods::ACCOUNT_ROTATE_PASSPHRASE,
        json!({ "account_id": "acct-main" }),
        &["old", "new"],
    ));
    assert_eq!(
        (rotate.old_passphrase.as_deref(), rotate.new_passphrase.as_str()),
        (Some("old"), "new")
    );
    let lock = request(call(methods::ACCOUNT_LOCK, json!({ "account_id": "acct-main" })));
    assert_eq!((lock.method.as_str(), lock.wire_body()), ("account.lock", json!({})));
    assert!(lock.operation_id.is_some());
}

#[test]
fn node_frames_name_the_node_and_a_pin_source_the_sdk_knows() {
    for (desktop, sdk) in [("Manual", "tofu"), ("Invite", "invitation"), ("Bootstrap", "builtin")] {
        let confirm =
            request(call(methods::NODE_CONFIRM, json!({ "node_id": "n", "pin_source": desktop })));
        let body = confirm.wire_body();
        assert_eq!(body["pin_source"], sdk);
        let _: NodePinSource = serde_json::from_value(body["pin_source"].clone())
            .unwrap_or_else(|e| panic!("{sdk} is not a pin source: {e}"));
        assert!(confirm.operation_id.is_some());
    }
    for method in [methods::NODE_REMOVE, methods::NODE_SET_DEFAULT, methods::NODE_SET_ENABLED] {
        let selector = request(call(method, json!({ "node_id": "n", "enabled": true })));
        assert_eq!(selector.wire_body()["node_id"], "n");
        assert!(selector.operation_id.is_some(), "{method} carries a retry key");
    }
    let probe = request(call(methods::NODE_PROBE, json!({ "node_id": "n" })));
    assert!(probe.operation_id.is_none(), "a probe changes nothing");
    let add = request(call(methods::NODE_ADD, json!({ "url": "https://n.example" })));
    assert_eq!(add.wire_body(), json!({ "url": "https://n.example" }));
}

#[test]
fn grant_conversation_device_and_group_frames_are_the_sdk_requests() {
    let approve: LocalBusMcpApprovalDecisionRequest =
        mutation(call(methods::GRANT_APPROVE, json!({ "grant_id": "appr-1" })));
    assert_eq!(approve.approval_id, "appr-1");
    let _: LocalBusMcpApprovalDecisionRequest =
        mutation(call(methods::GRANT_DENY, json!({ "grant_id": "appr-2" })));
    let revoke: LocalBusGrantRevokeRequest =
        mutation(call(methods::GRANT_REVOKE, json!({ "grant_id": "appr-1" })));
    assert_eq!(revoke.grant_id, "grant-1");
    let _: LocalBusGrantStandingApprovalRevokeRequest =
        mutation(call(methods::STANDING_REVOKE, json!({ "approval_id": "sa-1" })));
    let mute: LocalBusConversationMuteRequest =
        mutation(call(methods::CONTEXT_MUTE, json!({ "context_id": "dm-ana", "muted": true })));
    assert!(!mute.unmute);
    let ttl: LocalBusConversationDisappearingSetRequest = mutation(call(
        methods::CONTEXT_DISAPPEARING,
        json!({ "context_id": "dm-ana", "seconds": 60 }),
    ));
    assert_eq!(ttl.ttl_secs, 60);
    let _: LocalBusDeviceRevokeRequest =
        mutation(call(methods::DEVICE_REVOKE, json!({ "device_id": "device-x" })));
    // A group call carries the name this account keeps for the conversation
    // (the daemon records it from every group call that names one).
    let sent =
        keyed(call(methods::SEND_MESSAGE, json!({ "context_id": "thread-team", "text": "hi" })));
    let send: LocalBusGroupSendRequest = parses_with(&sent, &["title"]);
    assert_eq!(
        (send.group_id.as_str(), send.conversation_id.as_str(), send.sender_id.as_str()),
        ("g-team", "thread-team", "device-me"),
        "the group the conversation names; no group id is a conversation id"
    );
    assert!(
        send.target_delivery_id.is_none() && send.envelope_id.is_none(),
        "no delivery override: the daemon resolves every member's route"
    );
    let member = json!({ "context_id": "thread-team", "participant_id": "principal-bo" });
    let kick: LocalBusGroupMemberKickRequest = mutation(call(methods::GROUP_KICK, member.clone()));
    assert_eq!(
        (kick.member_id.as_str(), kick.actor_id.as_str()),
        ("device-bo", "device-me"),
        "the participant's member device"
    );
    let _: LocalBusGroupMemberBanRequest = mutation(call(methods::GROUP_BAN, member.clone()));
    let _: LocalBusGroupMemberRemoveRequest =
        mutation(call(methods::GROUP_REMOVE_MEMBER, member.clone()));
    let mut role = member;
    role["role"] = json!("Admin");
    let set: LocalBusGroupRoleSetRequest = mutation(call(methods::GROUP_SET_ROLE, role));
    assert_eq!(set.role, "admin");
}

#[test]
fn reads_and_pulls_are_the_sdk_requests_without_retry_keys() {
    let context = ContextId::new("dm-ana");
    let receive = Fetch::Receive(context.clone()).request(Some("acct-main"));
    let pulled: LocalBusMessageReceiveRequest = parses(&receive);
    assert_eq!(pulled.conversation_id.as_deref(), Some("dm-ana"));
    let group = group_receive().request(Some("acct-main"));
    let received: LocalBusGroupReceiveRequest = parses_with(&group, &["title"]);
    assert_eq!(group.wire_body()["title"], "Team");
    assert_eq!(
        (received.group_id.as_str(), received.conversation_id.as_str()),
        ("g-team", "thread-team")
    );
    let list = Fetch::Messages(context).request(Some("acct-main"));
    let _: LocalBusConversationRequest = parses(&list);
    for fetch in Fetch::account_snapshot(&AccountId::new("acct-main"))
        .into_iter()
        .chain([receive_of("dm-ana"), group_receive()])
    {
        assert!(fetch.request(Some("acct-main")).operation_id.is_none(), "{fetch:?}");
    }
    // Directory pages: the documented paging body, nothing else.
    for paged in [
        Paged::Identities,
        Paged::Conversations,
        Paged::Groups,
        Paged::Objects,
        Paged::Approvals,
        Paged::McpServers,
    ] {
        let first = Fetch::first(paged).request(Some("acct-main"));
        assert_eq!(first.wire_body(), json!({ "limit": 100, "cursor": null }), "{paged:?}");
        let next = Fetch::Page(paged, Some("c-1".to_owned())).request(Some("acct-main"));
        assert_eq!(next.wire_body(), json!({ "limit": 100, "cursor": "c-1" }));
        assert_eq!(next.account.as_deref(), Some("acct-main"));
    }
}

fn group_receive() -> Fetch {
    Fetch::GroupReceive {
        context: ContextId::new("thread-team"),
        group: "g-team".to_owned(),
        title: Some("Team".to_owned()),
    }
}

fn receive_of(context: &str) -> Fetch {
    Fetch::Receive(ContextId::new(context))
}

#[test]
fn each_effect_gets_its_own_retry_key() {
    let block = || request(call(methods::GRANT_DENY, json!({ "grant_id": "appr-2" })));
    let (first, second) = (block(), block());
    assert_ne!(first.operation_id, second.operation_id, "two effects are two operations");
}
