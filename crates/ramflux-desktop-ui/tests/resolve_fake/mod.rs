// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! One fake daemon with one unlocked account (`principal-me`) that resolves
//! contacts when they are added, as the contact-resolution contract has it,
//! and keeps groups the way the daemon's read model does.
//!
//! - The node's directory knows `principal-kai` (a person with one device)
//!   and `principal-lu` (no device listed: added, but not reachable).
//!   `contact.add {principal_id}` resolves a known principal provisionally
//!   (`root_source: lookup`; identity with devices and commitment; the row
//!   says `reachable` and `verified: false`) and answers `{contact}`; an
//!   unknown one is refused with `reason_code: unknown_principal`.
//! - `contact.add {card}` accepts `ramflux:contact/<principal>#<sig>` cards
//!   of known principals (`principal-bot` is an AI agent whose kind is
//!   self-declared) and pins their root (`root_source: card`); anything else
//!   is refused with `card_signature_invalid`. A card whose signature part
//!   starts with `other-root` names another root: for a contact looked up by
//!   id it is refused with `root_replacement_confirmation_required` and both
//!   roots, until `replace_provisional_root` names the looked-up root.
//! - `contact.verify` marks the contact verified (`root_source: verified`).
//! - `contact.card.export` answers this account's card and link.
//! - `group.create` records the group with its creator, conversation and
//!   name; `group.member.add` adds a member device; `group.list` and
//!   `conversation.directory` list them. Like the daemon, every group call
//!   that names a conversation records the name it carries and clears it
//!   when it carries none. `group.send` stores the message; `group.receive`
//!   answers the stored ones.
//! - `message.submit` starts a direct conversation with a resolved contact.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ramflux_desktop_test::fake_bus::{FakeBus, Reply, socket_path};
use serde_json::{Value, json};

/// This account's exported card.
pub const MY_CARD: &str = "ramflux:contact/principal-me#sig-me-0001";
/// This account's card link.
pub const MY_LINK: &str = "ramflux:contact/principal-me";

/// A principal the node's directory knows.
#[derive(Debug, Clone)]
pub struct Known {
    pub principal: &'static str,
    pub name: &'static str,
    pub kind: &'static str,
    pub devices: &'static [&'static str],
}

/// The node's directory.
pub const DIRECTORY: [Known; 3] = [
    Known { principal: "principal-kai", name: "Kai", kind: "human", devices: &["device-kai"] },
    Known { principal: "principal-lu", name: "Lu", kind: "human", devices: &[] },
    Known {
        principal: "principal-bot",
        name: "Helper",
        kind: "headless_ai",
        devices: &["device-bot"],
    },
];

fn known(principal: &str) -> Option<&'static Known> {
    DIRECTORY.iter().find(|k| k.principal == principal)
}

/// One group the account made.
#[derive(Debug, Clone, Default)]
pub struct Group {
    pub conversation: String,
    pub title: Value,
    pub members: Vec<String>,
    pub messages: Vec<Value>,
}

/// What the fake account holds.
#[derive(Default)]
pub struct State {
    /// Resolved contacts: principal, root commitment and `root_source`.
    pub contacts: Vec<(String, String, &'static str)>,
    /// Groups by group id.
    pub groups: BTreeMap<String, Group>,
    /// Direct conversations started, by id, with their peer.
    pub direct: BTreeMap<String, (String, Value)>,
    /// Every request seen, in order (method and body).
    pub seen: Vec<(String, Value)>,
    /// The account names no home node (`account.status` without
    /// `node_id`).
    pub without_home: bool,
}

/// The shared state.
pub type Shared = Arc<Mutex<State>>;

/// The contact entry the daemon answers and joins into its rows.
fn entry(principal: &str, root: &str, source: &str) -> Value {
    let who = known(principal);
    json!({
        "principal_id": principal, "display_name": who.map(|k| k.name),
        "reachable": who.is_some_and(|k| !k.devices.is_empty()),
        "verified": source == "verified",
        "kind_asserted": if source == "card" { who.map(|k| k.kind) } else { None },
        "root_commitment": root, "root_source": source, "node": null,
    })
}

/// A `contact.list` row: the link record joined with the contact entry.
fn link(principal: &str, root: &str, source: &str) -> Value {
    let mut row = json!({
        "link_id": format!("link-{principal}"), "requester_id": "principal-me",
        "target_id": root, "state": "accepted", "blocked": false,
    });
    if let (Value::Object(row), Value::Object(entry)) = (&mut row, entry(principal, root, source)) {
        row.extend(entry);
    }
    row
}

fn refused(reason: &'static str, details: Option<Value>) -> Reply {
    Reply::Refused {
        code: "ValidationFailed",
        message: reason.to_owned(),
        reason_code: reason,
        details,
    }
}

/// `contact.add {card}` or `{card, replace_provisional_root}`.
fn add_card(state: &mut State, body: &Value) -> Reply {
    let card = text(body, "card");
    let (principal, signature) = card
        .strip_prefix("ramflux:contact/")
        .and_then(|rest| rest.split_once('#'))
        .unwrap_or_default();
    if known(principal).is_none() {
        return refused("card_signature_invalid", None);
    }
    let root = if signature.starts_with("other-root") {
        format!("cm-other-{principal}")
    } else {
        format!("cm-{principal}")
    };
    let held = state.contacts.iter().find(|(p, ..)| p == principal).cloned();
    if let Some((_, previous, source)) = held
        && previous != root
    {
        let replace = text(body, "replace_provisional_root");
        if source != "lookup" {
            return refused("root_conflict", None);
        }
        if replace.is_empty() {
            let details = json!({
                "principal_id": principal, "previous_root_commitment": previous,
                "proposed_root_commitment": root,
            });
            return refused("root_replacement_confirmation_required", Some(details));
        }
        if replace != previous {
            return refused("root_conflict", None);
        }
    }
    state.contacts.retain(|(p, ..)| p != principal);
    state.contacts.push((principal.to_owned(), root.clone(), "card"));
    Reply::Ok(json!({ "contact": entry(principal, &root, "card") }))
}

fn identity(principal: &str, root: &str, source: &str) -> Option<Value> {
    let who = known(principal)?;
    let devices: Vec<Value> = who
        .devices
        .iter()
        .map(|d| {
            json!({ "id": d, "device_id": d, "label": null, "last_seen": null,
                    "verified_at": 1_790_000_000_u64,
                    "target_delivery_id": format!("target-{d}"),
                    "signing_public_key": format!("key-{d}") })
        })
        .collect();
    Some(json!({
        "id": principal, "principal_id": principal,
        "principal_commitment": root,
        "contact_identity_commitment": root,
        "display_name": who.name, "kind": if who.kind == "human" { Value::from("human") } else { Value::Null },
        "is_self": false, "devices": devices,
        "safety_state": if source == "verified" { "verified" } else { "unverified" },
        "safety_number": format!("{:0>60}", principal.len()), "safety_number_available": true,
        "root_source": source,
    }))
}

fn group_row(id: &str, group: &Group) -> Value {
    let mut roles = serde_json::Map::new();
    roles.insert("device-me".to_owned(), json!("owner"));
    let mut routes = vec![json!({
        "member_id": "device-me", "principal_id": "principal-me", "delivery_target_known": true,
    })];
    for member in &group.members {
        roles.insert(member.clone(), json!("member"));
        let principal =
            DIRECTORY.iter().find(|k| k.devices.contains(&member.as_str())).map(|k| k.principal);
        routes.push(json!({
            "member_id": member, "principal_id": principal, "delivery_target_known": true,
        }));
    }
    json!({ "group_id": id, "id": id, "group_epoch": 1, "roles": roles,
            "member_routes": routes, "conversation_id": group.conversation })
}

fn group_conversation(id: &str, group: &Group) -> Value {
    let mut participants = vec![json!("principal-me")];
    participants.extend(group.members.iter().filter_map(|m| {
        DIRECTORY.iter().find(|k| k.devices.contains(&m.as_str())).map(|k| json!(k.principal))
    }));
    json!({
        "id": group.conversation, "archived": false, "hidden": false, "unread_count": 0,
        "is_unread": false, "muted": false, "disappearing": null, "kind": "group",
        "title": group.title, "participants": participants, "group_id": id,
        "direct_route": null, "self_principal_id": "principal-me",
    })
}

fn decoded(body: &Value) -> String {
    ramflux_protocol::decode_base64url(body["plaintext_body_base64"].as_str().unwrap_or_default())
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_default()
}

fn text(body: &Value, field: &str) -> String {
    body[field].as_str().unwrap_or_default().to_owned()
}

fn handle(state: &mut State, all: &BTreeMap<String, Value>, method: &str, body: &Value) -> Reply {
    let fixture = || all.get(method).cloned().unwrap_or(json!({}));
    Reply::Ok(match method {
        "contact.list" => {
            let rows: Vec<Value> = state.contacts.iter().map(|(p, r, s)| link(p, r, s)).collect();
            json!({ "contacts": rows })
        }
        "account.status" if state.without_home => {
            let mut status = fixture();
            status.as_object_mut().map(|s| s.remove("node_id"));
            status
        }
        "contact.add" if body.get("card").is_some() => return add_card(state, body),
        "contact.add" => {
            let principal = text(body, "principal_id");
            if body.as_object().is_none_or(|b| b.len() != 1) {
                return refused("invalid_card", None);
            }
            if known(&principal).is_none() {
                return refused("unknown_principal", None);
            }
            if state.contacts.iter().any(|(p, ..)| p == &principal) {
                return refused("already_contact", None);
            }
            let root = format!("cm-{principal}");
            state.contacts.push((principal.clone(), root.clone(), "lookup"));
            json!({ "contact": entry(&principal, &root, "lookup") })
        }
        "contact.verify" => {
            let commitment = text(body, "contact_identity_commitment");
            for contact in &mut state.contacts {
                if contact.1 == commitment {
                    contact.2 = "verified";
                }
            }
            json!({ "verification_state": "verified" })
        }
        "contact.card.export" => json!({ "card": MY_CARD, "link": MY_LINK }),
        "identity.directory" => {
            let mut frame = all["identity.directory"].clone();
            let rows = frame["identities"].as_array_mut().expect("rows");
            rows.retain(|r| r["is_self"].as_bool().unwrap_or(false));
            rows.extend(state.contacts.iter().filter_map(|(p, r, s)| identity(p, r, s)));
            frame
        }
        "group.create" => {
            let id = text(body, "group_id");
            state.groups.insert(
                id.clone(),
                Group {
                    conversation: text(body, "conversation_id"),
                    title: body.get("title").cloned().unwrap_or(Value::Null),
                    ..Group::default()
                },
            );
            json!({ "group_id": id, "group_epoch": 1, "members": ["device-me"] })
        }
        "group.member.add" => {
            let Some(group) = state.groups.get_mut(&text(body, "group_id")) else {
                return Reply::Error("ValidationFailed", "no such group".to_owned());
            };
            group.members.push(text(body, "member_id"));
            json!({ "added": true })
        }
        "group.list" => {
            let rows: Vec<Value> = state.groups.iter().map(|(id, g)| group_row(id, g)).collect();
            json!({ "groups": rows, "next_cursor": null })
        }
        "group.send" | "group.receive" => {
            let Some(group) = state.groups.get_mut(&text(body, "group_id")) else {
                return Reply::Error("ValidationFailed", "no such group".to_owned());
            };
            // The daemon records the name every group call carries.
            group.title = body.get("title").cloned().unwrap_or(Value::Null);
            if method == "group.send" {
                group.messages.push(json!({
                    "conversation_id": group.conversation, "message_id": body["message_id"],
                    "sender_id": "device-me", "encrypted_body": decoded(body).as_bytes(),
                    "deleted": false, "created_at": 1_790_000_600, "receipts": [],
                }));
                json!({ "message_id": body["message_id"] })
            } else {
                json!({ "messages": group.messages, "decrypted_messages": [] })
            }
        }
        "conversation.directory" => {
            let mut frame = fixture();
            let rows = frame["conversations"].as_array_mut().expect("rows");
            rows.retain(|r| r["kind"] == "direct" && r["id"] == "dm-ana");
            rows.extend(state.groups.iter().map(|(id, g)| group_conversation(id, g)));
            rows.extend(state.direct.iter().map(|(id, (peer, title))| {
                json!({
                    "id": id, "archived": false, "hidden": false, "unread_count": 0,
                    "is_unread": false, "muted": false, "disappearing": null, "kind": "direct",
                    "title": title, "participants": ["principal-me", peer], "group_id": null,
                    "self_principal_id": "principal-me",
                    "direct_route": { "recipient_principal_commitment": format!("cm-{peer}"),
                                      "recipient_device_ids": [format!("device-{}", &peer[10..])],
                                      "target_delivery_id": format!("target-device-{}", &peer[10..]),
                                      "federation": null },
                })
            }));
            frame
        }
        "message.submit" => {
            let conversation = text(body, "conversation_id");
            let peer = DIRECTORY
                .iter()
                .find(|k| {
                    k.devices
                        .iter()
                        .any(|d| format!("target-{d}") == text(body, "target_delivery_id"))
                })
                .map(|k| k.principal.to_owned());
            let Some(peer) = peer else {
                return Reply::Error("ValidationFailed", "unknown delivery target".to_owned());
            };
            state.direct.entry(conversation).or_insert((peer, body["title"].clone()));
            json!({ "message_id": body["message_id"], "accepted": true })
        }
        "message.list" => json!({ "messages": [] }),
        "group.members" => json!({}),
        _ => fixture(),
    })
}

/// Starts the fake daemon at a socket named after `name`.
pub fn start(name: &str) -> (FakeBus, Shared, std::path::PathBuf) {
    start_with(name, false)
}

/// Starts the fake daemon; `without_home`: the account names no home node.
pub fn start_with(name: &str, without_home: bool) -> (FakeBus, Shared, std::path::PathBuf) {
    let path = socket_path(name);
    let shared: Shared = Arc::default();
    shared.lock().expect("state").without_home = without_home;
    let state = shared.clone();
    let all = crate::sdk_fake::frames();
    let fake = FakeBus::start(&path, move |method, _, body| {
        let mut state = state.lock().expect("state");
        state.seen.push((method.to_owned(), body.clone()));
        handle(&mut state, &all, method, body)
    });
    (fake, shared, path)
}
