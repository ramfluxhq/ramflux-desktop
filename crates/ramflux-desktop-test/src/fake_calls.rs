// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The fake daemon's voice calls: the CALL-01 §6 bus API, built ahead of
//! the daemon (contract-first), on the CALL-01 §2 signaling rules.
//!
//! Requests (account scoped):
//!
//! - `call.invite {conversation_id}` → the call row, `outgoing_ringing`.
//!   Refused (`Busy`) while this account has a call under way, and
//!   (`NotFound`) for a conversation the fake does not know.
//! - `call.answer {call_id}` → `connecting` (a call ringing here only).
//! - `call.reject {call_id}` → `ended`, reason `rejected`.
//! - `call.hangup {call_id}` → `ended`, reason `hangup`.
//! - `call.media.session {call_id}` → the relay endpoint, the opaque
//!   allocation token and both directions' SFrame keys (base64url); only
//!   once the call was answered. The fake takes the first media session as
//!   media flowing: the call becomes `active`.
//!
//! The call row (also the `call.changed` event body): `call_id`,
//! `conversation_id`, `peer_id`, `direction` (`outgoing` / `incoming`),
//! `state` (`outgoing_ringing`, `incoming_ringing`, `connecting`, `active`,
//! `ended`), `end_reason` (`hangup`, `rejected`, `busy`, `timeout`,
//! `answered_elsewhere`, `failed`; null until ended), `seq` (per call,
//! rising), `created_at`, `answered_at`, `ended_at` (Unix seconds).
//!
//! The other side is simulated: a test rings this account
//! ([`FakeCalls::incoming`]), answers, declines or hangs up as the peer, has
//! another device answer, and moves the fake clock past the 45-second ring
//! timeout ([`FakeCalls::advance`]). Every change is published as
//! `call.changed` through the publisher the test installs (the bus's event
//! journal and push subscribers).

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Value, json};

use crate::fake_bus::Reply;

/// Ring timeout (CALL-01 §2).
pub const RING_TIMEOUT: Duration = Duration::from_secs(45);
/// The fake clock's start (Unix seconds).
pub const FIRST_CALL_TIME: i64 = 1_790_000_000;
/// The SFrame suite the fake hands out (AES_256_GCM_SHA512_128).
pub const SUITE: u16 = 0x0005;

/// Delivers one event: `(account, topic, body)`.
pub type Publisher = Box<dyn Fn(&str, &str, Value) + Send + Sync>;

#[derive(Debug, Clone)]
struct Held {
    account: String,
    conversation: String,
    peer: String,
    outgoing: bool,
    state: &'static str,
    end_reason: Option<&'static str>,
    seq: u64,
    created_at: i64,
    answered_at: Option<i64>,
    ended_at: Option<i64>,
    /// Fake clock when it started ringing.
    rang_at: Duration,
}

impl Held {
    fn ended(&self) -> bool {
        self.state == "ended"
    }

    fn row(&self, id: &str) -> Value {
        json!({
            "call_id": id,
            "conversation_id": self.conversation,
            "peer_id": self.peer,
            "direction": if self.outgoing { "outgoing" } else { "incoming" },
            "state": self.state,
            "end_reason": self.end_reason,
            "seq": self.seq,
            "created_at": self.created_at,
            "answered_at": self.answered_at,
            "ended_at": self.ended_at,
        })
    }
}

#[derive(Default)]
struct Inner {
    calls: BTreeMap<String, Held>,
    /// Conversation id to the peer's principal.
    conversations: BTreeMap<String, String>,
    clock: Duration,
    next: u64,
    relay: String,
    media_sessions: Vec<String>,
}

impl Inner {
    fn now(&self) -> i64 {
        FIRST_CALL_TIME + i64::try_from(self.clock.as_secs()).unwrap_or(0)
    }

    fn fresh_id(&mut self) -> String {
        self.next += 1;
        format!("call-{:032x}", self.next)
    }

    fn under_way(&self, account: &str) -> bool {
        self.calls.values().any(|c| c.account == account && !c.ended())
    }

    /// Moves `id` to `state` (and `reason`); returns the event to publish.
    fn set(&mut self, id: &str, state: &'static str, reason: Option<&'static str>) -> Event {
        let now = self.now();
        let held = self.calls.get_mut(id).expect("known call");
        held.state = state;
        held.end_reason = reason;
        held.seq += 1;
        if state == "connecting" && held.answered_at.is_none() {
            held.answered_at = Some(now);
        }
        if state == "ended" {
            held.ended_at = Some(now);
        }
        (held.account.clone(), held.row(id))
    }
}

type Event = (String, Value);

/// Voice calls of the fake daemon (any number of accounts).
pub struct FakeCalls {
    inner: Mutex<Inner>,
    publisher: Mutex<Option<Publisher>>,
}

impl std::fmt::Debug for FakeCalls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeCalls").finish_non_exhaustive()
    }
}

impl Default for FakeCalls {
    fn default() -> Self {
        Self::new("203.0.113.7:3478")
    }
}

fn refused(code: &'static str, message: &str) -> Reply {
    Reply::Error(code, message.to_owned())
}

fn key(call: &str, direction: &str) -> String {
    // Deterministic per call and direction (a test reads them back); a
    // real daemon draws them at random (CALL-01 §3).
    let mut bytes = [0_u8; 32];
    for (i, b) in format!("{direction}/{call}").bytes().cycle().take(32).enumerate() {
        bytes[i] = b ^ (i as u8);
    }
    ramflux_protocol::encode_base64url(bytes)
}

impl FakeCalls {
    /// Calls whose media goes to `relay` (`host:port`).
    pub fn new(relay: impl Into<String>) -> Self {
        let inner = Inner { relay: relay.into(), ..Inner::default() };
        Self { inner: Mutex::new(inner), publisher: Mutex::new(None) }
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("fake calls")
    }

    /// Installs where `call.changed` events go.
    pub fn set_publisher(&self, publisher: Publisher) {
        *self.publisher.lock().expect("publisher") = Some(publisher);
    }

    fn publish(&self, events: Vec<Event>) {
        if let Some(publish) = self.publisher.lock().expect("publisher").as_ref() {
            for (account, row) in events {
                publish(&account, "call.changed", row);
            }
        }
    }

    /// Declares a direct conversation and its peer (calls are placed only
    /// in conversations the fake knows).
    pub fn conversation(&self, conversation: &str, peer: &str) {
        self.inner().conversations.insert(conversation.to_owned(), peer.to_owned());
    }

    /// The call row of `id`.
    pub fn row(&self, id: &str) -> Option<Value> {
        self.inner().calls.get(id).map(|held| held.row(id))
    }

    /// The newest call of `account`.
    pub fn last(&self, account: &str) -> Option<String> {
        let inner = self.inner();
        inner
            .calls
            .iter()
            .filter(|(_, c)| c.account == account)
            .max_by_key(|(id, c)| (c.created_at, (*id).clone()))
            .map(|(id, _)| id.clone())
    }

    /// Calls whose media session was asked for, in order.
    pub fn media_sessions(&self) -> Vec<String> {
        self.inner().media_sessions.clone()
    }

    /// Answers a call request; `None` for any other method.
    pub fn handle(&self, method: &str, account: Option<&str>, body: &Value) -> Option<Reply> {
        if !method.starts_with("call.") {
            return None;
        }
        let account = account.unwrap_or_default();
        let call_id = body["call_id"].as_str().unwrap_or_default();
        let (reply, events) = {
            let mut inner = self.inner();
            match method {
                "call.invite" => Self::invite(&mut inner, account, body),
                "call.media.session" => Self::media(&mut inner, account, call_id),
                "call.answer" | "call.reject" | "call.hangup" => {
                    Self::local(&mut inner, account, method, call_id)
                }
                _ => (refused("Unsupported", &format!("unsupported method {method}")), Vec::new()),
            }
        };
        self.publish(events);
        Some(reply)
    }

    fn invite(inner: &mut Inner, account: &str, body: &Value) -> (Reply, Vec<Event>) {
        let conversation = body["conversation_id"].as_str().unwrap_or_default();
        let Some(peer) = inner.conversations.get(conversation).cloned() else {
            return (refused("NotFound", "conversation not found"), Vec::new());
        };
        if inner.under_way(account) {
            return (refused("Busy", "another call is under way"), Vec::new());
        }
        let id = inner.fresh_id();
        let held = Held {
            account: account.to_owned(),
            conversation: conversation.to_owned(),
            peer,
            outgoing: true,
            state: "outgoing_ringing",
            end_reason: None,
            seq: 1,
            created_at: inner.now(),
            answered_at: None,
            ended_at: None,
            rang_at: inner.clock,
        };
        let row = held.row(&id);
        inner.calls.insert(id, held);
        (Reply::Ok(row.clone()), vec![(account.to_owned(), row)])
    }

    fn local(inner: &mut Inner, account: &str, method: &str, id: &str) -> (Reply, Vec<Event>) {
        let Some(held) = inner.calls.get(id).filter(|c| c.account == account) else {
            return (refused("NotFound", "call not found"), Vec::new());
        };
        let ringing_here = held.state == "incoming_ringing";
        let event = match method {
            "call.answer" if ringing_here => inner.set(id, "connecting", None),
            "call.reject" if ringing_here => inner.set(id, "ended", Some("rejected")),
            "call.hangup" if !held.ended() => inner.set(id, "ended", Some("hangup")),
            _ => return (refused("InvalidState", "the call is not in that state"), Vec::new()),
        };
        (Reply::Ok(event.1.clone()), vec![event])
    }

    fn media(inner: &mut Inner, account: &str, id: &str) -> (Reply, Vec<Event>) {
        let Some(held) = inner.calls.get(id).filter(|c| c.account == account) else {
            return (refused("NotFound", "call not found"), Vec::new());
        };
        if !matches!(held.state, "connecting" | "active") {
            return (refused("InvalidState", "the call is not connected"), Vec::new());
        }
        let (send, receive) =
            if held.outgoing { ("caller", "callee") } else { ("callee", "caller") };
        let session = json!({
            "call_id": id,
            "relay": {
                "endpoint": inner.relay,
                "token": ramflux_protocol::encode_base64url(format!("{{\"allocation\":\"{id}/{send}\"}}")),
                "expires_at": inner.now() + 600,
            },
            "sframe": {
                "cipher_suite": SUITE,
                "send": { "kid": if held.outgoing { 1 } else { 2 }, "base_key": key(id, send) },
                "receive": { "kid": if held.outgoing { 2 } else { 1 }, "base_key": key(id, receive) },
            },
        });
        inner.media_sessions.push(id.to_owned());
        let events = if held.state == "connecting" {
            vec![inner.set(id, "active", None)]
        } else {
            Vec::new()
        };
        (Reply::Ok(session), events)
    }

    /// The peer of `conversation` calls `account`. While `account` has a
    /// call under way the daemon answers the peer busy: the call is
    /// recorded here as ended (`busy`), a missed call. Returns the call id.
    pub fn incoming(&self, account: &str, conversation: &str) -> String {
        let (id, events) = {
            let mut inner = self.inner();
            let peer = inner.conversations.get(conversation).cloned().unwrap_or_default();
            let busy = inner.under_way(account);
            let id = inner.fresh_id();
            let now = inner.now();
            let held = Held {
                account: account.to_owned(),
                conversation: conversation.to_owned(),
                peer,
                outgoing: false,
                state: if busy { "ended" } else { "incoming_ringing" },
                end_reason: busy.then_some("busy"),
                seq: 1,
                created_at: now,
                answered_at: None,
                ended_at: busy.then_some(now),
                rang_at: inner.clock,
            };
            let row = held.row(&id);
            inner.calls.insert(id.clone(), held);
            (id, vec![(account.to_owned(), row)])
        };
        self.publish(events);
        id
    }

    fn peer(&self, id: &str, allowed: &[&str], state: &'static str, reason: Option<&'static str>) {
        let events = {
            let mut inner = self.inner();
            let current = inner.calls.get(id).map(|c| c.state);
            match current {
                Some(current) if allowed.contains(&current) => vec![inner.set(id, state, reason)],
                _ => Vec::new(),
            }
        };
        self.publish(events);
    }

    /// The peer answers an outgoing call.
    pub fn peer_answers(&self, id: &str) {
        self.peer(id, &["outgoing_ringing"], "connecting", None);
    }

    /// The peer declines an outgoing call.
    pub fn peer_rejects(&self, id: &str) {
        self.peer(id, &["outgoing_ringing"], "ended", Some("rejected"));
    }

    /// The peer is in another call.
    pub fn peer_busy(&self, id: &str) {
        self.peer(id, &["outgoing_ringing"], "ended", Some("busy"));
    }

    /// The peer hangs up (or cancels its ringing call).
    pub fn peer_hangs_up(&self, id: &str) {
        let live = ["outgoing_ringing", "incoming_ringing", "connecting", "active"];
        self.peer(id, &live, "ended", Some("hangup"));
    }

    /// Another device of this account answered the call ringing here.
    pub fn answered_elsewhere(&self, id: &str) {
        self.peer(id, &["incoming_ringing"], "ended", Some("answered_elsewhere"));
    }

    /// Signaling or media failed.
    pub fn fail(&self, id: &str) {
        let live = ["outgoing_ringing", "incoming_ringing", "connecting", "active"];
        self.peer(id, &live, "ended", Some("failed"));
    }

    /// Moves the fake clock; calls that rang for [`RING_TIMEOUT`] end
    /// (`timeout`).
    pub fn advance(&self, by: Duration) {
        let events = {
            let mut inner = self.inner();
            inner.clock += by;
            let clock = inner.clock;
            let due: Vec<String> = inner
                .calls
                .iter()
                .filter(|(_, c)| c.state.ends_with("_ringing") && clock - c.rang_at >= RING_TIMEOUT)
                .map(|(id, _)| id.clone())
                .collect();
            due.iter().map(|id| inner.set(id, "ended", Some("timeout"))).collect()
        };
        self.publish(events);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    type Seen = Arc<Mutex<Vec<(String, Value)>>>;

    fn calls() -> (FakeCalls, Seen) {
        let calls = FakeCalls::default();
        calls.conversation("dm-ana", "p-ana");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        calls.set_publisher(Box::new(move |account, topic, body| {
            assert_eq!(topic, "call.changed");
            sink.lock().unwrap().push((account.to_owned(), body));
        }));
        (calls, seen)
    }

    fn ok(reply: Option<Reply>) -> Value {
        match reply {
            Some(Reply::Ok(value)) => value,
            other => panic!("expected an answer, got {other:?}"),
        }
    }

    #[test]
    fn an_outgoing_call_rings_connects_and_hangs_up() {
        let (calls, seen) = calls();
        let invite = json!({ "conversation_id": "dm-ana" });
        let row = ok(calls.handle("call.invite", Some("a"), &invite));
        let id = row["call_id"].as_str().unwrap().to_owned();
        assert_eq!(row["state"], "outgoing_ringing");
        assert!(matches!(
            calls.handle("call.invite", Some("a"), &invite),
            Some(Reply::Error("Busy", _))
        ));
        calls.peer_answers(&id);
        let session = ok(calls.handle("call.media.session", Some("a"), &json!({ "call_id": id })));
        assert_eq!(session["sframe"]["cipher_suite"], 5);
        assert_eq!(session["sframe"]["send"]["kid"], 1);
        let hung = ok(calls.handle("call.hangup", Some("a"), &json!({ "call_id": id })));
        assert_eq!(
            (hung["state"].as_str(), hung["end_reason"].as_str()),
            (Some("ended"), Some("hangup"))
        );
        let states: Vec<String> = seen
            .lock()
            .unwrap()
            .iter()
            .map(|(_, r)| r["state"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(states, ["outgoing_ringing", "connecting", "active", "ended"]);
        let seqs: Vec<u64> =
            seen.lock().unwrap().iter().map(|(_, r)| r["seq"].as_u64().unwrap()).collect();
        assert_eq!(seqs, [1, 2, 3, 4]);
    }

    #[test]
    fn ringing_times_out_after_45_seconds_and_busy_is_a_missed_call() {
        let (calls, _) = calls();
        let id = calls.incoming("a", "dm-ana");
        calls.advance(Duration::from_secs(44));
        assert_eq!(calls.row(&id).unwrap()["state"], "incoming_ringing");
        calls.advance(Duration::from_secs(1));
        assert_eq!(calls.row(&id).unwrap()["end_reason"], "timeout");
        let first = calls.incoming("a", "dm-ana");
        let second = calls.incoming("a", "dm-ana");
        assert_eq!(calls.row(&first).unwrap()["state"], "incoming_ringing");
        assert_eq!(calls.row(&second).unwrap()["end_reason"], "busy");
        calls.answered_elsewhere(&first);
        assert_eq!(calls.row(&first).unwrap()["end_reason"], "answered_elsewhere");
        assert!(matches!(
            calls.handle("call.answer", Some("a"), &json!({ "call_id": first })),
            Some(Reply::Error("InvalidState", _))
        ));
    }
}
