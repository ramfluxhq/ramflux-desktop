// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The fake daemon's voice calls: the CALL-01 bus API as the daemon
//! implements it (product `docs/voice-calls.md`).
//!
//! Requests (account scoped):
//!
//! - `call.invite {conversation_id}` → the call row, `outgoing_ringing`.
//!   Refused (`Busy`) while this account has a call under way, and
//!   (`NotFound`) for a conversation the fake does not know.
//! - `call.answer {call_id}` → `connecting` (a call ringing here only);
//!   signaling then completes at once and the call becomes `active` (an
//!   event after the answer).
//! - `call.reject {call_id}` → `ended`, reason `declined`.
//! - `call.hangup {call_id}` → `ended`: `completed` once answered,
//!   `cancelled` by the caller before, `missed` by a ringing callee.
//! - `call.list {}` → `{calls: [rows]}`, newest first.
//! - `call.media.session {call_id}` → this side's relay endpoint, token,
//!   lease id, side and rebind key, and both directions' SFrame keys
//!   (base64url; KID 0 caller to callee, 1 back); only while the call is
//!   `active`, and only once per call (the keys start once).
//!
//! The call row (also the `call.changed` event body): `call_id`,
//! `conversation_id`, `peer_id`, `direction` (`outgoing` / `incoming`),
//! `state` (`outgoing_ringing`, `incoming_ringing`, `connecting`, `active`,
//! `ended`), `end_reason` (`completed`, `cancelled`, `declined`, `busy`,
//! `timeout`, `answered_elsewhere`, `failed`, `missed`; null until ended),
//! `seq` (per call, rising), `created_at`, `answered_at` (when it became
//! active), `ended_at` (Unix seconds), `media_generation` (0).
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
/// The fake clock's start: the wall clock when the fake was made (Unix
/// seconds), so durations a window measures against its own clock read as
/// they would with a daemon on the same machine.
fn first_call_time() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}
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
            "media_generation": 0,
        })
    }
}

#[derive(Default)]
struct Inner {
    calls: BTreeMap<String, Held>,
    /// Conversation id to the peer's principal.
    conversations: BTreeMap<String, String>,
    clock: Duration,
    start: i64,
    next: u64,
    relay: String,
    media_sessions: Vec<String>,
    /// Calls whose media session was handed out (once per call).
    media_taken: std::collections::BTreeSet<String>,
    /// A typed refusal for the local call decisions (answer, reject,
    /// hangup), as the daemon gives when a call is past its control receipt
    /// cap.
    refusal: Option<(&'static str, String)>,
}

impl Inner {
    fn now(&self) -> i64 {
        self.start + i64::try_from(self.clock.as_secs()).unwrap_or(0)
    }

    fn fresh_id(&mut self) -> String {
        self.next += 1;
        // Base64url of 16 bytes, as the daemon's (never starting with '-').
        let mut bytes = [0_u8; 16];
        bytes[8..].copy_from_slice(&self.next.to_be_bytes());
        bytes[0] = 0xc4;
        ramflux_protocol::encode_base64url(bytes)
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
        if state == "active" && held.answered_at.is_none() {
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
    /// Off: a daemon without the call API (every `call.*` is unknown).
    available: std::sync::atomic::AtomicBool,
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

/// Why a hangup ends `held`, as the daemon records it on this side: by
/// this account (`local`) or by the peer.
fn hangup_reason(held: &Held, local: bool) -> &'static str {
    match (held.answered_at.is_some(), held.outgoing, local) {
        (true, ..) => "completed",
        // This account cancelled its own call.
        (false, true, true) => "cancelled",
        // The callee hung up a call it had not answered.
        (false, true, false) => "declined",
        // A ringing call cancelled by its caller, or ignored here.
        (false, false, _) => "missed",
    }
}

fn lease_id(call: &str) -> [u8; 16] {
    let mut bytes = [0_u8; 16];
    for (i, b) in format!("lease/{call}").bytes().cycle().take(16).enumerate() {
        bytes[i] = b ^ (i as u8);
    }
    bytes
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
        let inner = Inner { relay: relay.into(), start: first_call_time(), ..Inner::default() };
        Self {
            inner: Mutex::new(inner),
            publisher: Mutex::new(None),
            available: std::sync::atomic::AtomicBool::new(true),
        }
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

    /// Makes the daemon refuse answer, reject and hangup with the typed
    /// `code` (for example `CallControlReceiptCapacity`), or accept them
    /// again (`None`).
    pub fn set_refusal(&self, refusal: Option<(&'static str, &str)>) {
        self.inner().refusal = refusal.map(|(code, message)| (code, message.to_owned()));
    }

    /// Makes the daemon one without the call API (`false`), or with it.
    pub fn set_available(&self, available: bool) {
        self.available.store(available, std::sync::atomic::Ordering::SeqCst);
    }

    /// Answers a call request; `None` for any other method.
    pub fn handle(&self, method: &str, account: Option<&str>, body: &Value) -> Option<Reply> {
        if !method.starts_with("call.") {
            return None;
        }
        if !self.available.load(std::sync::atomic::Ordering::SeqCst) {
            let message = format!("unsupported local bus method: {method}");
            return Some(refused("LocalBus", &message));
        }
        let account = account.unwrap_or_default();
        let call_id = body["call_id"].as_str().unwrap_or_default();
        let (reply, events) = {
            let mut inner = self.inner();
            match method {
                "call.invite" => Self::invite(&mut inner, account, body),
                "call.media.session" => Self::media(&mut inner, account, call_id),
                "call.list" => (Self::list(&inner, account), Vec::new()),
                "call.answer" | "call.reject" | "call.hangup" => {
                    if let Some((code, message)) = inner.refusal.clone() {
                        (Reply::Error(code, message), Vec::new())
                    } else {
                        Self::local(&mut inner, account, method, call_id)
                    }
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
        let reason = hangup_reason(held, true);
        let event = match method {
            "call.answer" if ringing_here => inner.set(id, "connecting", None),
            "call.reject" if ringing_here => inner.set(id, "ended", Some("declined")),
            "call.hangup" if !held.ended() => inner.set(id, "ended", Some(reason)),
            _ => return (refused("InvalidState", "the call is not in that state"), Vec::new()),
        };
        let reply = Reply::Ok(event.1.clone());
        let mut events = vec![event];
        if method == "call.answer" {
            // Signaling completes: the caller's ack reached this side.
            events.push(inner.set(id, "active", None));
        }
        (reply, events)
    }

    fn media(inner: &mut Inner, account: &str, id: &str) -> (Reply, Vec<Event>) {
        let Some(held) = inner.calls.get(id).filter(|c| c.account == account) else {
            return (refused("NotFound", "call not found"), Vec::new());
        };
        if held.state != "active" || inner.media_taken.contains(id) {
            let message =
                "media session unavailable; start a fresh call before restarting the media engine";
            return (refused("InvalidState", message), Vec::new());
        }
        let (side, send_kid) = if held.outgoing { (0, 0) } else { (1, 1) };
        let (send, receive) =
            if held.outgoing { ("caller", "callee") } else { ("callee", "caller") };
        let session = json!({
            "relay": {
                "endpoint": inner.relay,
                "token": key(id, &format!("token/{send}")),
                "lease_id": ramflux_protocol::encode_base64url(lease_id(id)),
                "side": side,
                "rebind_key": key(id, &format!("rebind/{send}")),
            },
            "sframe": {
                "cipher_suite": SUITE,
                "send": { "kid": send_kid, "base_key": key(id, send) },
                "receive": { "kid": send_kid ^ 1, "base_key": key(id, receive) },
            },
        });
        inner.media_sessions.push(id.to_owned());
        inner.media_taken.insert(id.to_owned());
        (Reply::Ok(session), Vec::new())
    }

    fn list(inner: &Inner, account: &str) -> Reply {
        let mut calls: Vec<(&String, &Held)> =
            inner.calls.iter().filter(|(_, c)| c.account == account).collect();
        calls.sort_by_key(|(id, c)| std::cmp::Reverse((c.created_at, (*id).clone())));
        let rows: Vec<Value> = calls.iter().take(128).map(|(id, c)| c.row(id)).collect();
        Reply::Ok(json!({ "calls": rows }))
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

    /// The peer answers an outgoing call; signaling completes and the call
    /// becomes active.
    pub fn peer_answers(&self, id: &str) {
        self.peer(id, &["outgoing_ringing"], "connecting", None);
        self.peer(id, &["connecting"], "active", None);
    }

    /// The peer declines an outgoing call.
    pub fn peer_rejects(&self, id: &str) {
        self.peer(id, &["outgoing_ringing"], "ended", Some("declined"));
    }

    /// The peer is in another call.
    pub fn peer_busy(&self, id: &str) {
        self.peer(id, &["outgoing_ringing"], "ended", Some("busy"));
    }

    /// The peer hangs up (or cancels its ringing call: a missed call here).
    pub fn peer_hangs_up(&self, id: &str) {
        let live = ["outgoing_ringing", "incoming_ringing", "connecting", "active"];
        let reason = self.inner().calls.get(id).map(|c| hangup_reason(c, false));
        self.peer(id, &live, "ended", reason);
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
            due.iter()
                .map(|id| {
                    let outgoing = inner.calls.get(id).is_some_and(|c| c.outgoing);
                    inner.set(id, "ended", Some(if outgoing { "timeout" } else { "missed" }))
                })
                .collect()
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
        let media = json!({ "call_id": id });
        let session = ok(calls.handle("call.media.session", Some("a"), &media));
        assert_eq!(session["sframe"]["cipher_suite"], 5);
        assert_eq!(session["sframe"]["send"]["kid"], 0);
        assert_eq!(session["relay"]["side"], 0);
        // Once per call: the keys start once.
        assert!(matches!(
            calls.handle("call.media.session", Some("a"), &media),
            Some(Reply::Error("InvalidState", _))
        ));
        let listed = ok(calls.handle("call.list", Some("a"), &json!({})));
        assert_eq!(listed["calls"][0]["state"], "active");
        let hung = ok(calls.handle("call.hangup", Some("a"), &json!({ "call_id": id })));
        assert_eq!(
            (hung["state"].as_str(), hung["end_reason"].as_str()),
            (Some("ended"), Some("completed"))
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
        assert_eq!(calls.row(&id).unwrap()["end_reason"], "missed");
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
