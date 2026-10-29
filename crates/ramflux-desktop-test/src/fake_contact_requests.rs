// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The fake daemon's contact requests (CONTACT-02 as merged on product
//! main 6158459; the bus contract the desktop assumes is in
//! docs/contract.md).
//!
//! Requests (account scoped):
//!
//! - `contact.requests.list {}` → `{requests: [row], contact_requests}`:
//!   the pending incoming requests and this account's own requests
//!   (pending first, then answered or expired ones).
//! - `contact.accept|decline|block {request_id}` (or `{principal_id}`:
//!   each selector matches only its own column) → `{request_id,
//!   principal_id, root_commitment, state, direction}`; an incoming
//!   request. The same answer again succeeds; another answer to an
//!   answered request is refused `request_closed`, an unknown one
//!   `request_not_found` (`ValidationFailed`). Accepting delivers the held
//!   messages (see [`FakeContactRequests::delivered`]).
//! - `contact.requests.policy.get {}` / `.set {contact_requests}` →
//!   `{contact_requests}` (`manual` or `auto_accept`).
//! - `contact.add {principal_id}` to a peer this account already asked
//!   ([`FakeContactRequests::outgoing`]): accepted → refused
//!   `already_contact`; pending → the same request sent again
//!   ([`FakeContactRequests::sent`]); expired, or declined after the
//!   cooldown ([`FakeContactRequests::cooldown_over`]) → a new request;
//!   declined within the 24 h cooldown → refused untyped
//!   (`ValidationFailed`), as the daemon does, also once the closed row
//!   is pruned from the list ([`FakeContactRequests::prune`]: the daemon
//!   keeps the declined status). Any other add is not answered here.
//! - The list can be refused like a locked account or an outage refuses
//!   it ([`FakeContactRequests::fail_list`]); a daemon without requests
//!   ([`FakeContactRequests::set_available`]) refuses every request method
//!   as the daemon refuses a method it does not know (`ValidationFailed`,
//!   "local bus error: unsupported local bus method: …").
//! - While the daemon is busy ([`FakeContactRequests::set_busy`]: its
//!   contact request capacity is exhausted), decisions and the setting are
//!   refused `Unavailable`, untyped (product `ContactRequestCapacity`).
//!
//! A row: `request_id`, `principal_id`, `root_commitment`, `direction`
//! (`inbound` / `outbound`), `state`, `note` and `preview` (quoted by the
//! product's own `quote_fallback_field`, exactly as the daemon sends them:
//! `"…"` with `\"`, `\n`, `\u{…}` escapes), `safety_number` (twelve digit
//! groups), `message_count`, `expires_at` (Unix seconds).
//!
//! The peer side is simulated: a test has a peer ask this account
//! ([`FakeContactRequests::incoming`]), hold messages from it
//! ([`FakeContactRequests::hold`]), and answer this account's own request
//! ([`FakeContactRequests::outgoing`], [`FakeContactRequests::peer_answers`]).
//! Every change is published (`contact.requested`, `contact.accepted`,
//! `contact.declined`, `contact.blocked`, `contact.request.expired`) with
//! identifiers and counts only (`request_id`, `principal_id`,
//! `root_commitment`, `state`, `direction`, `message_count`) through the
//! publisher the test installs; the note, preview and safety number are
//! read through the list. With the setting `auto_accept`, an incoming
//! request is accepted at once.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use ramflux_protocol::text_safety::quote_fallback_field;
use serde_json::{Value, json};

use crate::fake_bus::Reply;
use crate::fake_profiles::FakeProfiles;

/// Delivers one event: `(account, topic, body)`.
pub type Publisher = Box<dyn Fn(&str, &str, Value) + Send + Sync>;

#[derive(Debug, Clone)]
struct Held {
    account: String,
    peer: String,
    incoming: bool,
    state: &'static str,
    note: Option<String>,
    safety_number: Option<[&'static str; 12]>,
    messages: Vec<String>,
    /// Times this request was sent (an explicit add again resends it).
    sent: u32,
    /// A decline whose 24 h cooldown is over.
    cooled: bool,
    /// A closed request pruned from the list (an hour after it closed);
    /// its state stays on the contact row.
    pruned: bool,
}

impl Held {
    /// The list row, with what the daemon knows of the peer's public name
    /// and kind (as every request row carries it).
    fn row(&self, id: &str, profiles: &FakeProfiles) -> Value {
        let mut row = json!({
            "request_id": id,
            "principal_id": self.peer,
            "root_commitment": format!("root-{}", self.peer),
            "direction": if self.incoming { "inbound" } else { "outbound" },
            "state": self.state,
            // As the daemon writes them: delimited, sanitized quoted fields
            // (product `text_safety::quote_fallback_field`; the preview is
            // the first 256 characters of the first held message).
            "note": self.note.as_deref().map(quote_fallback_field),
            "safety_number": self.safety_number,
            "message_count": self.messages.len(),
            "preview": self.messages.first().map(|m| {
                quote_fallback_field(&m.chars().take(256).collect::<String>())
            }),
            "expires_at": 1_790_604_800,
        });
        profiles.join(&mut row, &self.peer);
        row
    }

    /// An event body: identifiers and counts only.
    fn event(&self, id: &str) -> Value {
        json!({
            "request_id": id,
            "principal_id": self.peer,
            "root_commitment": format!("root-{}", self.peer),
            "direction": if self.incoming { "inbound" } else { "outbound" },
            "state": self.state,
            "message_count": self.messages.len(),
        })
    }

    /// A decision answer.
    fn decision(&self, id: &str) -> Value {
        json!({
            "request_id": id,
            "principal_id": self.peer,
            "root_commitment": format!("root-{}", self.peer),
            "direction": if self.incoming { "inbound" } else { "outbound" },
            "state": self.state,
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
    /// Request capacity exhausted: decisions and the setting are refused
    /// `Unavailable`.
    busy: bool,
    /// Decisions and the setting are refused with this code and message
    /// (any daemon error frame, e.g. `TransportSecurity`).
    decision_failure: Option<(&'static str, String)>,
    /// The list is refused with this code and message (a locked account,
    /// an outage, a row the daemon could not read).
    list_failure: Option<(&'static str, String)>,
    /// The public profiles joined into rows.
    profiles: Arc<FakeProfiles>,
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

fn typed(reason_code: &'static str, message: &str) -> Reply {
    Reply::Refused {
        code: "ValidationFailed",
        message: message.to_owned(),
        reason_code,
        details: None,
    }
}

const SAFETY_NUMBER: [&str; 12] = [
    "05173", "88420", "61937", "24459", "90314", "77502", "13579", "24680", "11223", "44556",
    "77889", "99001",
];

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

    /// The public profiles the daemon joins into its rows (shared with the
    /// contact and identity rows of the same fake daemon).
    pub fn set_profiles(&self, profiles: Arc<FakeProfiles>) {
        self.inner().profiles = profiles;
    }

    /// The public profiles the daemon joins into its rows.
    pub fn profiles(&self) -> Arc<FakeProfiles> {
        self.inner().profiles.clone()
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
        let inner = self.inner();
        inner.requests.get(id).map(|held| held.row(id, &inner.profiles))
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
        if method == "contact.add" {
            return self.add(account.unwrap_or_default(), body);
        }
        let ours = matches!(
            method,
            "contact.requests.list"
                | "contact.requests.policy.get"
                | "contact.requests.policy.set"
                | "contact.accept"
                | "contact.decline"
                | "contact.block"
        );
        // contact.accept and contact.block name a request with a request
        // id or a principal id; other forms are the older link calls.
        let answer = matches!(method, "contact.accept" | "contact.decline" | "contact.block");
        let named = body.get("request_id").is_some() || body.get("principal_id").is_some();
        if !ours || (answer && !named) {
            return None;
        }
        if !self.available.load(Ordering::SeqCst) {
            // As the daemon refuses a method it does not know.
            let message = format!("local bus error: unsupported local bus method: {method}");
            return Some(refused("ValidationFailed", &message));
        }
        let account = account.unwrap_or_default().to_owned();
        let (reply, events) = {
            let mut inner = self.inner();
            if let (true, Some((code, message))) =
                (method == "contact.requests.list", inner.list_failure.clone())
            {
                return Some(refused(code, &message));
            }
            let read = matches!(method, "contact.requests.list" | "contact.requests.policy.get");
            if let (false, Some((code, message))) = (read, inner.decision_failure.clone()) {
                return Some(refused(code, &message));
            }
            if inner.busy && !read {
                let busy = refused("Unavailable", "contact request rate limit exceeded");
                return Some(busy);
            }
            match method {
                "contact.requests.list" => {
                    let mut rows: Vec<(bool, Value)> = inner
                        .requests
                        .iter()
                        .filter(|(_, r)| {
                            r.account == account
                                && !r.pruned
                                && (!r.incoming || r.state == "pending")
                        })
                        .map(|(id, r)| (r.state != "pending", r.row(id, &inner.profiles)))
                        .collect();
                    // Pending first (a stable sort keeps the order within).
                    rows.sort_by_key(|(closed, _)| *closed);
                    let rows: Vec<Value> = rows.into_iter().map(|(_, row)| row).collect();
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
        // Each selector matches only its own column.
        let found = inner.requests.iter().find(|(id, r)| {
            r.account == account
                && r.incoming
                && match (body["request_id"].as_str(), body["principal_id"].as_str()) {
                    (Some(wanted), _) => *id == wanted,
                    (None, Some(peer)) => r.peer == peer,
                    _ => false,
                }
        });
        let Some((id, request)) = found else {
            return (typed("request_not_found", "contact request not found"), Vec::new());
        };
        let id = id.clone();
        let (state, topic) = match method {
            "contact.accept" => ("accepted", "contact.accepted"),
            "contact.decline" => ("declined", "contact.declined"),
            _ => ("blocked", "contact.blocked"),
        };
        if request.state == state {
            return (Reply::Ok(request.decision(&id)), Vec::new());
        }
        // A block can still close an answered request.
        if request.state != "pending" && state != "blocked" {
            return (typed("request_closed", "contact request is closed"), Vec::new());
        }
        let event = Self::set(inner, &id, state, topic);
        let answer = inner.requests[&id].decision(&id);
        (Reply::Ok(answer), vec![event])
    }

    /// `contact.add` to a peer this account already asked; `None` for one
    /// it did not (the add is answered elsewhere).
    fn add(&self, account: &str, body: &Value) -> Option<Reply> {
        let peer = body["principal_id"].as_str()?;
        let mut inner = self.inner();
        let profiles = inner.profiles.clone();
        let (id, held) = inner
            .requests
            .iter_mut()
            .rev()
            .find(|(_, r)| r.account == account && r.peer == peer && !r.incoming)?;
        let id = id.clone();
        let reply = |id: &str, state: &str| {
            let mut contact =
                json!({ "principal_id": peer, "request_id": id, "request_state": state });
            profiles.join(&mut contact, peer);
            Reply::Ok(json!({ "contact": contact, "request_delivery": "submitted" }))
        };
        match held.state {
            "accepted" => Some(typed("already_contact", "already a contact")),
            "pending" => {
                held.sent += 1;
                Some(reply(&id, "pending"))
            }
            "declined" if !held.cooled => Some(refused(
                "ValidationFailed",
                "inbound rejected: contact request is closed or cooling down",
            )),
            _ => {
                let fresh = Self::fresh_id(&mut inner);
                inner.requests.insert(fresh.clone(), Self::asked(account, peer));
                Some(reply(&fresh, "pending"))
            }
        }
    }

    fn asked(account: &str, peer: &str) -> Held {
        Held {
            account: account.to_owned(),
            peer: peer.to_owned(),
            incoming: false,
            state: "pending",
            note: None,
            safety_number: Some(SAFETY_NUMBER),
            messages: Vec::new(),
            sent: 1,
            cooled: false,
            pruned: false,
        }
    }

    /// Makes the daemon's contact request capacity exhausted (`true`) or
    /// free again.
    pub fn set_busy(&self, busy: bool) {
        self.inner().busy = busy;
    }

    /// Refuses decisions and the setting with `code` and `message` (any
    /// daemon error frame), or answers them again (`None`).
    pub fn fail_decisions(&self, failure: Option<(&'static str, &str)>) {
        self.inner().decision_failure = failure.map(|(code, message)| (code, message.to_owned()));
    }

    /// Refuses `contact.requests.list` with `code` and `message` (as the
    /// daemon does for a locked account, an outage or a row it cannot
    /// read), or lists again (`None`).
    pub fn fail_list(&self, failure: Option<(&'static str, &str)>) {
        self.inner().list_failure = failure.map(|(code, message)| (code, message.to_owned()));
    }

    /// The daemon prunes closed request `id` from the list (an hour after
    /// it closed); the contact row keeps its state, and a decline still
    /// cools down.
    pub fn prune(&self, id: &str) {
        if let Some(held) = self.inner().requests.get_mut(id).filter(|r| r.state != "pending") {
            held.pruned = true;
        }
    }

    /// How many times request `id` was sent.
    pub fn sent(&self, id: &str) -> u32 {
        self.inner().requests.get(id).map_or(0, |r| r.sent)
    }

    /// The 24 h cooldown of a decline of `account`'s request `id` is over.
    pub fn cooldown_over(&self, id: &str) {
        if let Some(held) = self.inner().requests.get_mut(id) {
            held.cooled = true;
        }
    }

    fn set(inner: &mut Inner, id: &str, state: &'static str, topic: &'static str) -> Event {
        let held = inner.requests.get_mut(id).expect("known request");
        held.state = state;
        let messages = std::mem::take(&mut held.messages);
        let (account, peer) = (held.account.clone(), held.peer.clone());
        let row = held.event(id);
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
                safety_number: Some(SAFETY_NUMBER),
                messages: Vec::new(),
                sent: 1,
                cooled: false,
                pruned: false,
            };
            let mut events = vec![(account.to_owned(), "contact.requested", held.event(&id))];
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
    /// the new count (the preview is read through the list).
    pub fn hold(&self, id: &str, text: &str) {
        let events = {
            let mut inner = self.inner();
            let Some(held) = inner.requests.get_mut(id).filter(|r| r.state == "pending") else {
                return;
            };
            held.messages.push(text.to_owned());
            vec![(held.account.clone(), "contact.requested", held.event(id))]
        };
        self.publish(events);
    }

    /// `account` asked `peer` (what `contact.add` does on a CONTACT-02
    /// daemon); returns the request id.
    pub fn outgoing(&self, account: &str, peer: &str) -> String {
        let mut inner = self.inner();
        let id = Self::fresh_id(&mut inner);
        inner.requests.insert(id.clone(), Self::asked(account, peer));
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
        // Quoted as the daemon quotes them.
        assert_eq!(listed["requests"][0]["preview"], "\"hi, it's Bo\"");
        assert_eq!(listed["requests"][0]["note"], "\"from the meetup\"");
        assert_eq!(listed["contact_requests"], "manual");
        let answer = ok(fake.handle("contact.accept", Some("a"), &json!({ "request_id": id })));
        assert_eq!(answer["state"], "accepted");
        assert_eq!(fake.delivered(), [("a".into(), "p-bo".into(), "hi, it's Bo".into())]);
        // The same answer again succeeds; another one is refused.
        let again = ok(fake.handle("contact.accept", Some("a"), &json!({ "request_id": id })));
        assert_eq!(again["state"], "accepted");
        assert!(matches!(
            fake.handle("contact.decline", Some("a"), &json!({ "request_id": id })),
            Some(Reply::Refused { reason_code: "request_closed", .. })
        ));
        assert!(matches!(
            fake.handle("contact.decline", Some("a"), &json!({ "request_id": "nope" })),
            Some(Reply::Refused { reason_code: "request_not_found", .. })
        ));
        // Events carry identifiers and counts only.
        for (_, body) in seen.lock().unwrap().iter() {
            assert!(body.get("preview").is_none() && body.get("note").is_none(), "{body}");
            assert!(body.get("safety_number").is_none(), "{body}");
        }
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

    #[test]
    fn a_busy_daemon_refuses_decisions_unavailable_and_still_lists() {
        let fake = FakeContactRequests::default();
        let id = fake.incoming("a", "p-bo", None);
        fake.set_busy(true);
        assert!(matches!(
            fake.handle("contact.accept", Some("a"), &json!({ "request_id": id })),
            Some(Reply::Error("Unavailable", _))
        ));
        let listed = ok(fake.handle("contact.requests.list", Some("a"), &json!({})));
        assert_eq!(listed["requests"][0]["state"], "pending");
        fake.set_busy(false);
        let answer = ok(fake.handle("contact.accept", Some("a"), &json!({ "request_id": id })));
        assert_eq!(answer["state"], "accepted");
    }

    #[test]
    fn adding_again_retries_a_pending_request_and_respects_a_declines_cooldown() {
        let fake = FakeContactRequests::default();
        let add = json!({ "principal_id": "p-ana" });
        assert!(fake.handle("contact.add", Some("a"), &add).is_none(), "not asked yet");
        let id = fake.outgoing("a", "p-ana");
        let again = ok(fake.handle("contact.add", Some("a"), &add));
        assert_eq!(again["contact"]["request_id"], json!(id));
        assert_eq!(fake.sent(&id), 2);
        let listed = ok(fake.handle("contact.requests.list", Some("a"), &json!({})));
        assert_eq!(listed["requests"][0]["direction"], "outbound");

        fake.peer_answers(&id, "declined");
        assert!(matches!(
            fake.handle("contact.add", Some("a"), &add),
            Some(Reply::Error("ValidationFailed", _))
        ));
        // Pruned from the list, the decline still cools down and stays on
        // the contact row.
        fake.prune(&id);
        let listed = ok(fake.handle("contact.requests.list", Some("a"), &json!({})));
        assert_eq!(listed["requests"], json!([]));
        assert_eq!(fake.own_state("a", "p-ana"), Some("declined"));
        assert!(matches!(
            fake.handle("contact.add", Some("a"), &add),
            Some(Reply::Error("ValidationFailed", _))
        ));
        fake.cooldown_over(&id);
        let fresh = ok(fake.handle("contact.add", Some("a"), &add));
        assert_ne!(fresh["contact"]["request_id"], json!(id));
        let fresh = fresh["contact"]["request_id"].as_str().unwrap().to_owned();
        fake.peer_answers(&fresh, "accepted");
        assert!(matches!(
            fake.handle("contact.add", Some("a"), &add),
            Some(Reply::Refused { reason_code: "already_contact", .. })
        ));
    }
}
