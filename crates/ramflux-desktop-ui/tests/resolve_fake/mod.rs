// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! One fake daemon with one unlocked account (`principal-me`) that resolves
//! contacts when they are added, as the contact-resolution contract has it,
//! and keeps groups the way the daemon's read model does.
//!
//! - The node's directory knows `principal-kai` (a person with one device)
//!   and `principal-lu` (no device listed: added, but not reachable).
//!   `contact.add {principal_id}` resolves a known principal (identity with
//!   devices and commitment; the link record says `reachable` and
//!   `verified: false`); an unknown one fails as the node answers.
//! - `contact.card.import {card}` accepts `ramflux:contact/<principal>`
//!   cards of known principals (`principal-bot` is an AI agent whose kind is
//!   self-declared) and refuses anything else with a signature error.
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
    /// Resolved contacts: principal and how they were added.
    pub contacts: Vec<(String, bool)>,
    /// Groups by group id.
    pub groups: BTreeMap<String, Group>,
    /// Direct conversations started, by id, with their peer.
    pub direct: BTreeMap<String, (String, Value)>,
    /// Every request seen, in order (method and body).
    pub seen: Vec<(String, Value)>,
}

/// The shared state.
pub type Shared = Arc<Mutex<State>>;

fn link(principal: &str, by_card: bool) -> Value {
    let who = known(principal);
    json!({
        "link_id": format!("link-{principal}"), "requester_id": "principal-me",
        "target_id": principal, "state": "accepted", "blocked": false,
        "reachable": who.is_some_and(|k| !k.devices.is_empty()),
        "verified": false,
        "kind_asserted": if by_card { who.map(|k| k.kind) } else { None },
    })
}

fn identity(principal: &str) -> Option<Value> {
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
        "principal_commitment": format!("cm-{principal}"),
        "contact_identity_commitment": format!("cm-{principal}"),
        "display_name": who.name, "kind": if who.kind == "human" { Value::from("human") } else { Value::Null },
        "is_self": false, "devices": devices,
        "safety_state": "unverified", "safety_number": null, "safety_number_available": true,
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
            json!({ "contacts": state.contacts.iter().map(|(p, card)| link(p, *card)).collect::<Vec<_>>() })
        }
        "contact.add" => {
            let principal = text(body, "principal_id");
            if known(&principal).is_none() {
                return Reply::Error(
                    "ValidationFailed",
                    format!("{principal} is not known to its home node"),
                );
            }
            state.contacts.retain(|(p, _)| p != &principal);
            state.contacts.push((principal.clone(), false));
            link(&principal, false)
        }
        "contact.card.import" => {
            let card = text(body, "card");
            let principal = card.strip_prefix("ramflux:contact/").unwrap_or_default();
            let principal = principal.split('#').next().unwrap_or_default().to_owned();
            if known(&principal).is_none() {
                return Reply::Error(
                    "ValidationFailed",
                    "contact card signature does not verify".to_owned(),
                );
            }
            state.contacts.retain(|(p, _)| p != &principal);
            state.contacts.push((principal.clone(), true));
            json!({ "link": link(&principal, true) })
        }
        "contact.card.export" => json!({ "card": MY_CARD, "link": MY_LINK }),
        "identity.directory" => {
            let mut frame = all["identity.directory"].clone();
            let rows = frame["identities"].as_array_mut().expect("rows");
            rows.retain(|r| r["is_self"].as_bool().unwrap_or(false));
            rows.extend(state.contacts.iter().filter_map(|(p, _)| identity(p)));
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
        "group.members" | "contact.verify" => json!({}),
        _ => fixture(),
    })
}

/// Starts the fake daemon at a socket named after `name`.
pub fn start(name: &str) -> (FakeBus, Shared, std::path::PathBuf) {
    let path = socket_path(name);
    let shared: Shared = Arc::default();
    let state = shared.clone();
    let all = crate::sdk_fake::frames();
    let fake = FakeBus::start(&path, move |method, _, body| {
        let mut state = state.lock().expect("state");
        state.seen.push((method.to_owned(), body.clone()));
        handle(&mut state, &all, method, body)
    });
    (fake, shared, path)
}
