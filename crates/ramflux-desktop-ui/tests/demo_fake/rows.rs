// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The daemon's JSON rows, in the shapes the client read model lists them
//! (see the recorded frames in the SDK's test fixtures).

use serde_json::{Value, json};

use super::world::{Account, Conversation, Group, NodeRecord};

/// A node profile (`node.add` answer's `node`, `node.confirm`, `node.list`).
pub fn node_profile(node: &NodeRecord) -> Value {
    json!({
        "node_id": node.id, "discovery_url": node.url, "node_public_key": node.key,
        "pin_source": "tofu",
        "gateway": { "quic_addr": "gateway.node.invalid:7443", "tcp_addr": "", "server_name": "gateway.node.invalid" },
        "relay": { "quic_addr": "relay.node.invalid:7543", "server_name": "relay.node.invalid" },
        "prekey_http_url": "", "signaling_turn_addr": "", "protocol_versions": ["ramflux.v1"],
        "client_capabilities": ["gateway", "relay", "prekey"],
        "discovered_at": 1_790_000_000_u64, "expires_at": 1_890_000_000_u64,
        "enabled": true, "builtin": false, "safety_warnings": ["tofu_unverified"],
    })
}

/// `account.status` of an open account.
pub fn account_status(account: &Account, node: &str) -> Value {
    json!({
        "local_account_id": account.id, "node_id": node, "principal_id": account.principal,
        "principal_commitment": account.commitment, "target_delivery_id": account.target,
        "session_id": "session", "active_transport_kind": "quic", "pending_deliveries": 0,
    })
}

/// What `account.create` and `account.unlock` answer.
pub fn opened(account: &Account) -> Value {
    json!({
        "local_account_id": account.id, "principal_id": account.principal,
        "principal_commitment": account.commitment, "device_id": account.device,
        "target_delivery_id": account.target, "client_mode": "attended_cli",
        "session_id": "session", "active_transport_kind": "quic",
    })
}

/// `device.list` of an account (its one device).
pub fn devices(account: &Account) -> Value {
    json!({
        "principal_id": account.principal, "local_device_id": account.device,
        "devices": [{
            "device_id": account.device, "device_epoch": 1, "target_delivery_id": account.target,
            "capability_scope": ["device.delivery.bind"], "is_local": true,
        }],
    })
}

/// An identity row of `who` as `viewer` sees it.
pub fn identity(who: &Account, viewer: &Account) -> Value {
    let is_self = who.principal == viewer.principal;
    let verified = viewer.verified.contains(&who.commitment);
    json!({
        "id": who.principal, "principal_id": who.principal,
        "principal_commitment": who.commitment, "contact_identity_commitment": who.commitment,
        "display_name": who.label, "kind": "human", "is_self": is_self,
        "devices": [{
            "id": who.device, "device_id": who.device, "label": null, "last_seen": null,
            "verified_at": 1_790_000_000_u64, "target_delivery_id": who.target,
            "signing_public_key": format!("key-{}", who.device),
        }],
        "device_set_hash": format!("set-{}", who.device),
        "safety_state": if verified { "verified" } else { "unverified" },
        // Resolving a contact caches the safety number (CONTACT-01), so it
        // is listed to compare before it is verified.
        "safety_number": if is_self { Value::Null } else { json!("31415 92653 58979 32384") },
        "safety_number_available": !is_self,
    })
}

/// A conversation row of `owner`.
pub fn conversation(id: &str, conversation: &Conversation, owner: &Account) -> Value {
    let peer = &conversation.peer;
    json!({
        "id": id, "archived": false, "hidden": false, "last_activity": null,
        "unread_count": 0, "is_unread": false, "muted": false, "mute_until": null,
        "disappearing": null, "kind": "direct", "title": conversation.title,
        "participants": [peer.principal, owner.principal], "group_id": null,
        "direct_route": {
            "recipient_principal_commitment": peer.commitment,
            "recipient_device_ids": [peer.device],
            "target_delivery_id": peer.target, "federation": null,
        },
        "self_principal_id": owner.principal,
    })
}

/// An accepted friend link of a resolving daemon: the contact's devices
/// are held (reachable), the safety number not compared yet.
pub fn link(id: &Value, requester: &Value, target: &Value, root_source: &str) -> Value {
    json!({
        "link_id": id, "requester_id": requester, "target_id": target, "state": "accepted",
        "remove_scope": null, "blocked": false, "capability_revoked_at": null,
        "principal_id": target, "reachable": true, "verified": false, "kind_asserted": null,
        "root_source": root_source,
    })
}

/// A `group.list` row.
pub fn group(id: &str, group: &Group) -> Value {
    let roles: serde_json::Map<String, Value> = group
        .members
        .iter()
        .enumerate()
        .map(|(ix, (device, _))| (device.clone(), json!(if ix == 0 { "owner" } else { "member" })))
        .collect();
    let routes: Vec<Value> = group
        .members
        .iter()
        .map(|(device, principal)| {
            json!({ "member_id": device, "principal_id": principal, "delivery_target_known": true })
        })
        .collect();
    json!({
        "group_id": id, "id": id, "group_epoch": 1, "roles": roles, "member_routes": routes,
        "conversation_id": group.conversation, "conversation_ids": [group.conversation],
    })
}

/// The conversation row of `group` as `owner` sees it (with the name its
/// own daemon keeps).
pub fn group_conversation(id: &str, group: &Group, owner: &Account) -> Value {
    let participants: Vec<&String> = group.members.iter().map(|(_, p)| p).collect();
    json!({
        "id": group.conversation, "archived": false, "hidden": false, "last_activity": null,
        "unread_count": 0, "is_unread": false, "muted": false, "mute_until": null,
        "disappearing": null, "kind": "group",
        "title": group.titles.get(&owner.principal).cloned().unwrap_or(Value::Null),
        "participants": participants, "group_id": id, "direct_route": null,
        "self_principal_id": owner.principal,
    })
}

/// A stored message.
pub fn message(id: &str, sender: &str, text: &str, at: u64, delivered: bool) -> Value {
    json!({
        "message_id": id, "sender_id": sender, "encrypted_body": text.as_bytes(),
        "created_at": at, "deleted": false,
        "receipts": if delivered { json!([{ "state": "delivered" }]) } else { json!([]) },
    })
}
