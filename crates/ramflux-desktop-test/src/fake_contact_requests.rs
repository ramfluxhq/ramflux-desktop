// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The fake daemon's contact requests (CONTACT-02, contract-first; the bus
//! contract the desktop assumes is in docs/contract.md).
//!
//! Requests (account scoped):
//!
//! - `contact.requests.list {}` → `{requests: [row], contact_requests}`:
//!   the pending requests both ways and the account's setting.
//! - `contact.accept|decline|block {request_id}` → `{request_id,
//!   principal_id, state, direction}`; only a pending incoming request.
//!   Accepting delivers the held messages (see [`FakeContactRequests::delivered`]).
//! - `contact.requests.policy.get {}` / `.set {contact_requests}` →
//!   `{contact_requests}` (`manual` or `auto_accept`).
//!
//! A row: `request_id`, `principal_id`, `direction` (`inbound` /
//! `outbound`), `state`, `note`, `safety_number`, `message_count`,
//! `preview`, `expires_at` (Unix seconds).
//!
//! The peer side is simulated: a test has a peer ask this account
//! ([`FakeContactRequests::incoming`]), hold messages from it
//! ([`FakeContactRequests::hold`]), and answer this account's own request
//! ([`FakeContactRequests::outgoing`], [`FakeContactRequests::peer_answers`]).
//! Every change is published (`contact.requested`, `contact.accepted`,
//! `contact.declined`, `contact.blocked`, `contact.request.expired`) with
//! the row as body through the publisher the test installs. With the
//! setting `auto_accept`, an incoming request is accepted at once.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use crate::fake_bus::Reply;

/// Delivers one event: `(account, topic, body)`.
pub type Publisher = Box<dyn Fn(&str, &str, Value) + Send + Sync>;

#[derive(Debug, Clone)]
struct Held {
    account: String,
    peer: String,
    incoming: bool,
    state: &'static str,
    note: Option<String>,
    safety_number: String,
    messages: Vec<String>,
}

impl Held {
    fn row(&self, id: &str) -> Value {
        json!({
            "request_id": id,
            "principal_id": self.peer,
            "direction": if self.incoming { "inbound" } else { "outbound" },
            "state": self.state,
            "note": self.note,
            "safety_number": self.safety_number,
            "message_count": self.messages.len(),
            "preview": self.messages.first(),
            "expires_at": 1_790_604_800,
        })
    }
}

#[derive(Default)]
struct Inner {
    requests: BTreeMap<String, Held>,
    policy: BTreeMap<String, &'static str>,
    next: u64,
    /// Messages delivered into conversations on accept: (account, peer, text).
    delivered: Vec<(String, String, String)>,
}

/// Contact requests of the fake daemon (any number of accounts).
pub struct FakeContactRequests {
    inner: Mutex<Inner>,
    publisher: Mutex<Option<Publisher>>,
    available: AtomicBool,
}

impl Default for FakeContactRequests {
    fn default() -> Self {
        Self {
            inner: Mutex::default(),
            publisher: Mutex::new(None),
            available: AtomicBool::new(true),
        }
    }
}

impl std::fmt::Debug for FakeContactRequests {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeContactRequests").finish_non_exhaustive()
    }
}

fn refused(code: &'static str, message: &str) -> Reply {
    Reply::Error(code, message.to_owned())
}

type Event = (String, &'static str, Value);

impl FakeContactRequests {
    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("fake contact requests")
    }

    /// Installs where request events go.
    pub fn set_publisher(&self, publisher: Publisher) {
        *self.publisher.lock().expect("publisher") = Some(publisher);
    }

    /// Makes the daemon one without contact requests (`false`), or with.
    pub fn set_available(&self, available: bool) {
        self.available.store(available, Ordering::SeqCst);
    }

    fn publish(&self, events: Vec<Event>) {
        if let Some(publish) = self.publisher.lock().expect("publisher").as_ref() {
            for (account, topic, row) in events {
                publish(&account, topic, row);
            }
        }
    }

    /// The row of request `id`.
    pub fn row(&self, id: &str) -> Option<Value> {
        self.inner().requests.get(id).map(|held| held.row(id))
    }

    /// Messages delivered into conversations on accept: (account, peer,
    /// text), in order.
    pub fn delivered(&self) -> Vec<(String, String, String)> {
        self.inner().delivered.clone()
    }

    /// `account`'s setting (`manual` until set).
    pub fn policy(&self, account: &str) -> &'static str {
        self.inner().policy.get(account).copied().unwrap_or("manual")
    }

    /// Answers a request call; `None` for any other method.
    pub fn handle(&self, method: &str, account: Option<&str>, body: &Value) -> Option<Reply> {
        let ours = matches!(
            method,
            "contact.requests.list"
                | "contact.requests.policy.get"
                | "contact.requests.policy.set"
                | "contact.accept"
                | "contact.decline"
                | "contact.block"
        );
        // contact.accept and contact.block name a request only with a
        // request id; other forms are the older link calls.
        let answer = matches!(method, "contact.accept" | "contact.decline" | "contact.block");
        if !ours || (answer && body.get("request_id").is_none()) {
            return None;
        }
        if !self.available.load(Ordering::SeqCst) {
            return Some(refused("LocalBus", &format!("unsupported local bus method: {method}")));
        }
        let account = account.unwrap_or_default().to_owned();
        let (reply, events) = {
            let mut inner = self.inner();
            match method {
                "contact.requests.list" => {
                    let rows: Vec<Value> = inner
                        .requests
                        .iter()
                        .filter(|(_, r)| r.account == account && r.state == "pending")
                        .map(|(id, r)| r.row(id))
                        .collect();
                    let policy = inner.policy.get(&account).copied().unwrap_or("manual");
                    (Reply::Ok(json!({ "requests": rows, "contact_requests": policy })), Vec::new())
                }
                "contact.requests.policy.get" => {
                    let policy = inner.policy.get(&account).copied().unwrap_or("manual");
                    (Reply::Ok(json!({ "contact_requests": policy })), Vec::new())
                }
                "contact.requests.policy.set" => match body["contact_requests"].as_str() {
                    Some("manual") => {
                        inner.policy.insert(account.clone(), "manual");
                        (Reply::Ok(json!({ "contact_requests": "manual" })), Vec::new())
                    }
                    Some("auto_accept") => {
                        inner.policy.insert(account.clone(), "auto_accept");
                        (Reply::Ok(json!({ "contact_requests": "auto_accept" })), Vec::new())
                    }
                    _ => (refused("ValidationFailed", "unknown contact_requests"), Vec::new()),
                },
                _ => Self::decide(&mut inner, &account, method, body),
            }
        };
        self.publish(events);
        Some(reply)
    }

    fn decide(inner: &mut Inner, account: &str, method: &str, body: &Value) -> (Reply, Vec<Event>) {
        let id = body["request_id"].as_str().unwrap_or_default().to_owned();
        let pending = inner
            .requests
            .get(&id)
            .is_some_and(|r| r.account == account && r.incoming && r.state == "pending");
        if !pending {
            return (refused("NotFound", "contact request not found"), Vec::new());
        }
        let (state, topic) = match method {
            "contact.accept" => ("accepted", "contact.accepted"),
            "contact.decline" => ("declined", "contact.declined"),
            _ => ("blocked", "contact.blocked"),
        };
        let event = Self::set(inner, &id, state, topic);
        (Reply::Ok(event.2.clone()), vec![event])
    }

    fn set(inner: &mut Inner, id: &str, state: &'static str, topic: &'static str) -> Event {
        let held = inner.requests.get_mut(id).expect("known request");
        held.state = state;
        let messages = std::mem::take(&mut held.messages);
        let (account, peer) = (held.account.clone(), held.peer.clone());
        let row = held.row(id);
        if state == "accepted" {
            inner
                .delivered
                .extend(messages.into_iter().map(|m| (account.clone(), peer.clone(), m)));
        }
        (account, topic, row)
    }

    fn fresh_id(inner: &mut Inner) -> String {
        inner.next += 1;
        format!("rq{:04}", inner.next)
    }

    /// `peer` asks `account` to become contacts, with `note`; returns the
    /// request id. With the setting `auto_accept` it is accepted at once.
    pub fn incoming(&self, account: &str, peer: &str, note: Option<&str>) -> String {
        let (id, events) = {
            let mut inner = self.inner();
            let id = Self::fresh_id(&mut inner);
            let held = Held {
                account: account.to_owned(),
                peer: peer.to_owned(),
                incoming: true,
                state: "pending",
                note: note.map(str::to_owned),
                safety_number: "05173 88420 61937 24459 90314 77502".to_owned(),
                messages: Vec::new(),
            };
            let mut events = vec![(account.to_owned(), "contact.requested", held.row(&id))];
            inner.requests.insert(id.clone(), held);
            if inner.policy.get(account) == Some(&"auto_accept") {
                events.push(Self::set(&mut inner, &id, "accepted", "contact.accepted"));
            }
            (id, events)
        };
        self.publish(events);
        id
    }

    /// The requester of `id` sends `text` before an answer: the daemon holds
    /// it (a message request). Published as `contact.requested` again with
    /// the new count and preview.
    pub fn hold(&self, id: &str, text: &str) {
        let events = {
            let mut inner = self.inner();
            let Some(held) = inner.requests.get_mut(id).filter(|r| r.state == "pending") else {
                return;
            };
            held.messages.push(text.to_owned());
            vec![(held.account.clone(), "contact.requested", held.row(id))]
        };
        self.publish(events);
    }

    /// `account` asked `peer` (what `contact.add` does on a CONTACT-02
    /// daemon); returns the request id.
    pub fn outgoing(&self, account: &str, peer: &str) -> String {
        let mut inner = self.inner();
        let id = Self::fresh_id(&mut inner);
        inner.requests.insert(
            id.clone(),
            Held {
                account: account.to_owned(),
                peer: peer.to_owned(),
                incoming: false,
                state: "pending",
                note: None,
                safety_number: String::new(),
                messages: Vec::new(),
            },
        );
        id
    }

    /// The `request_state` of `account`'s own request to `peer`, as
    /// `contact.list` reports it on the contact's row.
    pub fn own_state(&self, account: &str, peer: &str) -> Option<&'static str> {
        self.inner()
            .requests
            .values()
            .rev()
            .find(|r| r.account == account && r.peer == peer && !r.incoming)
            .map(|r| r.state)
    }

    /// The peer answers `account`'s own request `id`: `accepted`,
    /// `declined` or `expired`.
    pub fn peer_answers(&self, id: &str, state: &'static str) {
        let events = {
            let mut inner = self.inner();
            let topic = match state {
                "accepted" => "contact.accepted",
                "declined" => "contact.declined",
                _ => "contact.request.expired",
            };
            if !inner.requests.get(id).is_some_and(|r| !r.incoming && r.state == "pending") {
                return;
            }
            vec![Self::set(&mut inner, id, state, topic)]
        };
        self.publish(events);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn ok(reply: Option<Reply>) -> Value {
        match reply {
            Some(Reply::Ok(value)) => value,
            other => panic!("expected an answer, got {other:?}"),
        }
    }

    #[test]
    fn a_request_is_listed_held_and_accepted_with_its_messages() {
        let fake = FakeContactRequests::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        fake.set_publisher(Box::new(move |_, topic, row| {
            sink.lock().unwrap().push((topic.to_owned(), row));
        }));
        let id = fake.incoming("a", "p-bo", Some("from the meetup"));
        fake.hold(&id, "hi, it's Bo");
        let listed = ok(fake.handle("contact.requests.list", Some("a"), &json!({})));
        assert_eq!(listed["requests"][0]["message_count"], 1);
        assert_eq!(listed["requests"][0]["preview"], "hi, it's Bo");
        assert_eq!(listed["contact_requests"], "manual");
        let answer = ok(fake.handle("contact.accept", Some("a"), &json!({ "request_id": id })));
        assert_eq!(answer["state"], "accepted");
        assert_eq!(fake.delivered(), [("a".into(), "p-bo".into(), "hi, it's Bo".into())]);
        assert!(matches!(
            fake.handle("contact.decline", Some("a"), &json!({ "request_id": id })),
            Some(Reply::Error("NotFound", _))
        ));
        let topics: Vec<String> = seen.lock().unwrap().iter().map(|(t, _)| t.clone()).collect();
        assert_eq!(topics, ["contact.requested", "contact.requested", "contact.accepted"]);
        // The older link form is not a request call.
        assert!(fake.handle("contact.accept", Some("a"), &json!({ "link_id": "l" })).is_none());
    }

    #[test]
    fn auto_accept_answers_at_once_and_declines_drop_held_messages() {
        let fake = FakeContactRequests::default();
        let set = json!({ "contact_requests": "auto_accept" });
        ok(fake.handle("contact.requests.policy.set", Some("a"), &set));
        let id = fake.incoming("a", "p-bo", None);
        assert_eq!(fake.row(&id).unwrap()["state"], "accepted");
        let other = fake.incoming("b", "p-cy", None);
        fake.hold(&other, "spam");
        ok(fake.handle("contact.decline", Some("b"), &json!({ "request_id": other })));
        assert!(fake.delivered().is_empty());
        assert_eq!(fake.row(&other).unwrap()["message_count"], 0);
    }
}
