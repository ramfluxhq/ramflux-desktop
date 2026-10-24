// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A fake demo node with two daemons on it: node discovery and pinning,
//! accounts with passphrases, one-way contact links resolved when they are
//! added (by principal id or from a contact card, as the contact-resolution
//! contract has it), direct messages held by the node until the recipient
//! receives them, delivery receipts once received, attachments listed on
//! the sender's side only (a received attachment is not fetched), groups
//! with their conversation (each member's daemon keeps the name its own
//! group calls carry), and a restart that locks a daemon's accounts.

use std::collections::{BTreeMap, BTreeSet};

use ramflux_desktop_test::fake_bus::Reply;
use serde_json::{Value, json};

use super::rows;

/// The node both daemons talk to.
#[derive(Debug, Clone)]
pub struct NodeRecord {
    /// Node id.
    pub id: String,
    /// Discovery URL.
    pub url: String,
    /// Public key.
    pub key: String,
}

/// Who is on the other side of a direct conversation.
#[derive(Debug, Clone)]
pub struct Peer {
    pub principal: String,
    pub commitment: String,
    pub device: String,
    pub target: String,
}

/// A direct conversation of one account.
#[derive(Debug, Clone)]
pub struct Conversation {
    pub peer: Peer,
    /// The local title the first submission named (the daemon records it).
    pub title: Option<String>,
    pub messages: Vec<Value>,
}

/// A local account of one daemon.
#[derive(Debug, Clone)]
pub struct Account {
    pub id: String,
    pub label: String,
    pub principal: String,
    pub commitment: String,
    pub device: String,
    pub target: String,
    secret: String,
    pub open: bool,
    pub links: Vec<Value>,
    pub conversations: BTreeMap<String, Conversation>,
    pub objects: Vec<Value>,
    pub verified: BTreeSet<String>,
}

impl Account {
    fn peer(&self) -> Peer {
        Peer {
            principal: self.principal.clone(),
            commitment: self.commitment.clone(),
            device: self.device.clone(),
            target: self.target.clone(),
        }
    }
}

/// A message the node holds for its recipient.
#[derive(Debug, Clone)]
struct Held {
    to_target: String,
    conversation: String,
    message: String,
    from: Peer,
    text: String,
    at: u64,
}

/// One daemon's state.
#[derive(Debug, Default)]
struct Daemon {
    confirmed: bool,
    accounts: Vec<Account>,
}

/// A group on the node.
#[derive(Debug, Clone, Default)]
pub struct Group {
    /// Its conversation.
    pub conversation: String,
    /// Member devices with their principals, owner first.
    pub members: Vec<(String, String)>,
    /// The name each member's daemon keeps, by principal.
    pub titles: BTreeMap<String, Value>,
    /// Messages: id, sender device, text, time.
    pub messages: Vec<(String, String, String, u64)>,
}

/// The node and both daemons.
#[derive(Debug)]
pub struct World {
    node: NodeRecord,
    daemons: [Daemon; 2],
    held: Vec<Held>,
    groups: BTreeMap<String, Group>,
    clock: u64,
}

/// The contact card of `account` (the fake's own format).
pub fn card_of(account: &Account) -> String {
    format!("ramflux:contact/{}#sig-{}", account.principal, account.device)
}

fn text(body: &Value, field: &str) -> String {
    body[field].as_str().unwrap_or_default().to_owned()
}

fn decoded(base64: &str) -> String {
    ramflux_protocol::decode_base64url(base64)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_default()
}

impl World {
    /// A node with `key` at `url`.
    pub fn new(url: &str, key: &str) -> Self {
        let node =
            NodeRecord { id: "demo-node".to_owned(), url: url.to_owned(), key: key.to_owned() };
        Self {
            node,
            daemons: Default::default(),
            held: Vec::new(),
            groups: BTreeMap::new(),
            clock: 1_790_000_100,
        }
    }

    /// Restarting daemon `side` closes its accounts.
    pub fn restart(&mut self, side: usize) {
        for account in &mut self.daemons[side].accounts {
            account.open = false;
        }
    }

    /// The pinned node's key on daemon `side`.
    pub fn node_key(&self, side: usize, node: &str) -> Option<String> {
        (self.daemons[side].confirmed && node == self.node.id).then(|| self.node.key.clone())
    }

    fn account(&mut self, side: usize, id: &str) -> Option<&mut Account> {
        self.daemons[side].accounts.iter_mut().find(|a| a.id == id)
    }

    fn by_principal(&self, principal: &str) -> Option<&Account> {
        self.daemons.iter().flat_map(|d| &d.accounts).find(|a| a.principal == principal)
    }

    fn by_device(&self, device: &str) -> Option<&Account> {
        self.daemons.iter().flat_map(|d| &d.accounts).find(|a| a.device == device)
    }

    /// The groups `principal` is a member of.
    fn groups_of(&self, principal: &str) -> Vec<(String, Group)> {
        self.groups
            .iter()
            .filter(|(_, g)| g.members.iter().any(|(_, p)| p == principal))
            .map(|(id, g)| (id.clone(), g.clone()))
            .collect()
    }

    fn by_target(&self, target: &str) -> Option<&Account> {
        self.daemons.iter().flat_map(|d| &d.accounts).find(|a| a.target == target)
    }

    /// Answers one request to daemon `side`.
    pub fn handle(
        &mut self,
        side: usize,
        method: &str,
        account: Option<&str>,
        body: &Value,
    ) -> Reply {
        if let Some(reply) = self.daemon_scope(side, method, body) {
            return reply;
        }
        let Some(id) = account.map(str::to_owned) else {
            return Reply::Ok(json!({}));
        };
        match method {
            "account.unlock" => return self.unlock(side, &id, body),
            "account.status" | "account.lock" | "device.list" => {}
            _ if method.starts_with("mcp.") || method.starts_with("grant.") => {
                return Reply::Ok(json!({}));
            }
            _ => {}
        }
        if !self.account(side, &id).is_some_and(|a| a.open) {
            return Reply::Error("account_locked", format!("account not open: {id}"));
        }
        self.account_scope(side, &id, method, body)
    }

    fn daemon_scope(&mut self, side: usize, method: &str, body: &Value) -> Option<Reply> {
        let daemon = &mut self.daemons[side];
        let open = daemon.accounts.iter().filter(|a| a.open).count();
        Some(Reply::Ok(match method {
            "daemon.status" => json!({ "accounts": open, "socket_path": "fake" }),
            "node.list" => {
                let nodes: Vec<Value> =
                    daemon.confirmed.then(|| rows::node_profile(&self.node)).into_iter().collect();
                let default = daemon.confirmed.then(|| self.node.id.clone());
                json!({ "default_node_id": default, "nodes": nodes })
            }
            "node.add" if text(body, "url") == self.node.url => {
                json!({ "node": rows::node_profile(&self.node), "persisted": false })
            }
            "node.add" => return Some(Reply::Error("not_found", "no client record".to_owned())),
            "node.confirm" => {
                daemon.confirmed = true;
                rows::node_profile(&self.node)
            }
            "account.list" => {
                let active = daemon.accounts.iter().find(|a| a.open).map(|a| a.id.clone());
                let ids: Vec<&str> =
                    daemon.accounts.iter().filter(|a| a.open).map(|a| a.id.as_str()).collect();
                let details: Vec<Value> = daemon
                    .accounts
                    .iter()
                    .map(|a| {
                        json!({
                            "id": a.id, "label": a.label, "kind": "human",
                            "principal_id": a.principal,
                            "state": if a.open { "unlocked" } else { "locked" },
                        })
                    })
                    .collect();
                json!({ "active_account_id": active, "accounts": ids, "account_details": details })
            }
            "account.create" => {
                let account = Account {
                    id: text(body, "local_account_id"),
                    label: text(body, "label"),
                    principal: text(body, "principal_id"),
                    commitment: text(body, "principal_commitment"),
                    device: text(body, "device_id"),
                    target: text(body, "target_delivery_id"),
                    secret: text(body, "account_secret"),
                    open: true,
                    links: Vec::new(),
                    conversations: BTreeMap::new(),
                    objects: Vec::new(),
                    verified: BTreeSet::new(),
                };
                let answer = rows::opened(&account);
                daemon.accounts.push(account);
                answer
            }
            _ => return None,
        }))
    }

    fn unlock(&mut self, side: usize, id: &str, body: &Value) -> Reply {
        let passphrase = text(body, "passphrase");
        match self.account(side, id) {
            Some(account) if account.secret == passphrase => {
                account.open = true;
                Reply::Ok(rows::opened(account))
            }
            Some(_) => Reply::Error("unauthorized", "wrong passphrase".to_owned()),
            None => Reply::Error("not_found", format!("no account {id}")),
        }
    }

    fn account_scope(&mut self, side: usize, id: &str, method: &str, body: &Value) -> Reply {
        let node = self.node.id.clone();
        let me = self.account(side, id).cloned().expect("open account");
        Reply::Ok(match method {
            "account.status" => rows::account_status(&me, &node),
            "account.lock" => {
                self.account(side, id).expect("account").open = false;
                json!({ "local_account_id": id, "locked": true })
            }
            "device.list" => rows::devices(&me),
            "identity.directory" => {
                let mut principals: BTreeSet<String> = BTreeSet::from([me.principal.clone()]);
                for link in &me.links {
                    principals.insert(text(link, "requester_id"));
                    principals.insert(text(link, "target_id"));
                }
                principals.extend(me.conversations.values().map(|c| c.peer.principal.clone()));
                let rows: Vec<Value> = principals
                    .iter()
                    .filter_map(|p| self.by_principal(p))
                    .map(|who| rows::identity(who, &me))
                    .collect();
                json!({ "identities": rows, "next_cursor": null })
            }
            "conversation.directory" => {
                let mut rows: Vec<Value> = me
                    .conversations
                    .iter()
                    .map(|(cid, c)| rows::conversation(cid, c, &me))
                    .collect();
                rows.extend(
                    self.groups_of(&me.principal)
                        .iter()
                        .map(|(gid, g)| rows::group_conversation(gid, g, &me)),
                );
                json!({ "conversations": rows, "next_cursor": null })
            }
            "object.directory" => json!({ "objects": me.objects, "next_cursor": null }),
            "group.list" => {
                let rows: Vec<Value> = self
                    .groups_of(&me.principal)
                    .iter()
                    .map(|(gid, g)| rows::group(gid, g))
                    .collect();
                json!({ "groups": rows, "next_cursor": null })
            }
            "contact.list" => json!({ "contacts": me.links }),
            "contact.add" if body.get("card").is_none() => {
                let principal = text(body, "principal_id");
                if self.by_principal(&principal).is_none() {
                    return Reply::Refused {
                        code: "ValidationFailed",
                        message: "unknown_principal".to_owned(),
                        reason_code: "unknown_principal",
                        details: None,
                    };
                }
                let link = rows::link(
                    &json!(format!("link-{principal}")),
                    &json!(me.principal),
                    &json!(principal),
                    "lookup",
                );
                self.account(side, id).expect("account").links.push(link.clone());
                json!({ "contact": link })
            }
            "contact.card.export" => {
                json!({ "card": card_of(&me), "link": format!("ramflux:contact/{}", me.principal) })
            }
            "contact.add" => {
                let card = text(body, "card");
                let named = self
                    .daemons
                    .iter()
                    .flat_map(|d| &d.accounts)
                    .find(|a| {
                        card_of(a) == card || format!("ramflux:contact/{}", a.principal) == card
                    })
                    .map(|a| a.principal.clone());
                let Some(principal) = named else {
                    return Reply::Refused {
                        code: "ValidationFailed",
                        message: "card_signature_invalid".to_owned(),
                        reason_code: "card_signature_invalid",
                        details: None,
                    };
                };
                let link = rows::link(
                    &json!(format!("link-card-{principal}")),
                    &json!(me.principal),
                    &json!(principal),
                    "card",
                );
                self.account(side, id).expect("account").links.push(link.clone());
                json!({ "contact": link })
            }
            "group.create" => {
                let gid = text(body, "group_id");
                let mut group = Group {
                    conversation: text(body, "conversation_id"),
                    members: vec![(me.device.clone(), me.principal.clone())],
                    ..Group::default()
                };
                group.titles.insert(me.principal.clone(), body["title"].clone());
                self.groups.insert(gid.clone(), group);
                json!({ "group_id": gid, "group_epoch": 1, "members": [me.device] })
            }
            "group.member.add" => {
                let device = text(body, "member_id");
                let principal = self.by_device(&device).map(|a| a.principal.clone());
                let (Some(group), Some(principal)) =
                    (self.groups.get_mut(&text(body, "group_id")), principal)
                else {
                    return Reply::Error("ValidationFailed", "unknown group or member".to_owned());
                };
                group.members.push((device, principal));
                json!({ "added": true })
            }
            "group.send" | "group.receive" => {
                self.clock += 1;
                let at = self.clock;
                let Some(group) = self.groups.get_mut(&text(body, "group_id")) else {
                    return Reply::Error("ValidationFailed", "unknown group".to_owned());
                };
                // Each daemon records the name its own group calls carry.
                group.titles.insert(me.principal.clone(), body["title"].clone());
                if method == "group.send" {
                    let message = text(body, "message_id");
                    group.messages.push((
                        message.clone(),
                        me.device.clone(),
                        decoded(body["plaintext_body_base64"].as_str().unwrap_or_default()),
                        at,
                    ));
                    json!({ "message_id": message })
                } else {
                    let messages: Vec<Value> = group
                        .messages
                        .iter()
                        .map(|(m, sender, text, at)| rows::message(m, sender, text, *at, false))
                        .collect();
                    // The daemon decrypts what it received for this member.
                    let decrypted: Vec<Value> = group
                        .messages
                        .iter()
                        .filter(|(_, sender, _, _)| sender != &me.device)
                        .map(|(m, _, text, _)| json!({ "message_id": m, "body_utf8": text }))
                        .collect();
                    json!({ "messages": messages, "decrypted_messages": decrypted })
                }
            }
            "contact.verify" => {
                let commitment = text(body, "contact_identity_commitment");
                self.account(side, id).expect("account").verified.insert(commitment);
                json!({})
            }
            "message.submit" => return self.submit(side, id, body),
            "message.receive" => self.receive(side, id, &text(body, "conversation_id")),
            "message.list" => {
                let messages = me
                    .conversations
                    .get(&text(body, "conversation_id"))
                    .map(|c| c.messages.clone())
                    .unwrap_or_default();
                json!({ "messages": messages })
            }
            _ => json!({}),
        })
    }

    /// A direct message from account `id` on daemon `side`, held by the
    /// node for its recipient.
    pub fn submit(&mut self, side: usize, id: &str, body: &Value) -> Reply {
        let Some(recipient) = self.by_target(&text(body, "target_delivery_id")).map(Account::peer)
        else {
            return Reply::Error("no_route", "unknown delivery target".to_owned());
        };
        self.clock += 1;
        let at = self.clock;
        let (conversation, message) = (text(body, "conversation_id"), text(body, "message_id"));
        let body_text = decoded(body["plaintext_body_base64"].as_str().unwrap_or_default());
        let me = self.account(side, id).expect("account");
        let from = me.peer();
        let stored = rows::message(&message, &from.device, &body_text, at, false);
        let title = body["title"].as_str().map(str::to_owned);
        me.conversations
            .entry(conversation.clone())
            .or_insert_with(|| Conversation {
                peer: recipient.clone(),
                title,
                messages: Vec::new(),
            })
            .messages
            .push(stored);
        for attachment in body["attachments"].as_array().into_iter().flatten() {
            let bytes = ramflux_protocol::decode_base64url(
                attachment["plaintext_base64"].as_str().unwrap_or_default(),
            )
            .map_or(0, |b| b.len());
            me.objects.push(json!({
                "id": attachment["object_id"], "size": bytes, "transfer_state": "available",
                "updated_at": at, "name": attachment["name"], "mime": null,
                "direction": "outgoing", "owner": from.principal,
                "conversation_id": conversation, "message_id": message,
                "relay_endpoint": attachment["relay_endpoint"], "transfer": null,
            }));
        }
        self.held.push(Held {
            to_target: recipient.target,
            conversation,
            message: message.clone(),
            from,
            text: body_text,
            at,
        });
        Reply::Ok(json!({ "message_id": message, "accepted": true }))
    }

    /// Takes the node's held messages of `conversation` for account `id`;
    /// the senders' copies become delivered.
    pub fn receive(&mut self, side: usize, id: &str, conversation: &str) -> Value {
        let target = self.account(side, id).map(|a| a.target.clone()).unwrap_or_default();
        let (mine, rest): (Vec<Held>, Vec<Held>) = std::mem::take(&mut self.held)
            .into_iter()
            .partition(|h| h.to_target == target && h.conversation == conversation);
        self.held = rest;
        let mut decrypted = Vec::new();
        for held in mine {
            let me = self.account(side, id).expect("account");
            me.conversations
                .entry(held.conversation.clone())
                .or_insert_with(|| Conversation {
                    peer: held.from.clone(),
                    title: None,
                    messages: Vec::new(),
                })
                .messages
                .push(rows::message(&held.message, &held.from.device, &held.text, held.at, false));
            decrypted.push(json!({ "message_id": held.message, "body_utf8": held.text }));
            self.mark_delivered(&held);
        }
        json!({ "entries": [], "decrypted_messages": decrypted })
    }

    fn mark_delivered(&mut self, held: &Held) {
        let sender = self
            .daemons
            .iter_mut()
            .flat_map(|d| d.accounts.iter_mut())
            .find(|a| a.principal == held.from.principal);
        let Some(conversation) = sender.and_then(|a| a.conversations.get_mut(&held.conversation))
        else {
            return;
        };
        for message in &mut conversation.messages {
            if message["message_id"] == held.message.as_str() {
                message["receipts"] = json!([{ "state": "delivered" }]);
            }
        }
    }
}
