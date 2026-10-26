// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The fake daemon's event journal and its streams (`bus.subscribe`), as
//! the daemon keeps them: per account, entries numbered from 1, a floor
//! below which entries are no longer kept, replay after a cursor, then live
//! delivery in order.
//!
//! Like the daemon, a cursor below the floor or above the head is refused
//! with `resync_required`, and a stream it stops (lag) ends with a final
//! `stream.lagged` frame. A fake that has the stream switched off refuses
//! `bus.subscribe` as an older daemon does.

use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

/// The connection writer a stream answers on.
pub type Writer = Arc<Mutex<UnixStream>>;

#[derive(Debug, Clone)]
struct Entry {
    seq: u64,
    topic: String,
    body: Value,
}

#[derive(Debug, Default)]
struct Account {
    entries: Vec<Entry>,
    head: u64,
}

impl Account {
    /// The oldest entry kept, or the head when none is.
    fn floor(&self) -> u64 {
        self.entries.first().map_or(self.head, |e| e.seq)
    }
}

struct Follower {
    connection: usize,
    account: String,
    topics: Vec<String>,
    writer: Writer,
    delivered: u64,
}

/// What a `bus.subscribe` gets.
#[derive(Debug, Clone, PartialEq)]
pub enum Subscribed {
    /// An error frame: code and message.
    Refused(&'static str, String),
    /// Accepted; the response, replay and live delivery are written.
    Following,
}

/// The journal of every account and the open streams.
#[derive(Default)]
pub struct Journal {
    enabled: bool,
    /// The daemon-level job journal: one stream without an account (its
    /// key is empty and its frames carry no account).
    daemon: bool,
    accounts: BTreeMap<String, Account>,
    followers: Vec<Follower>,
}

pub(crate) fn write_frame(stream: &mut UnixStream, frame: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(frame).expect("frame json");
    let len = u32::try_from(body.len()).expect("frame size");
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&body)
}

/// The frame's account: none for the daemon-level journal (empty key).
fn account_field(account: &str) -> Value {
    if account.is_empty() { Value::Null } else { json!(account) }
}

fn event_frame(account: &str, seq: Option<u64>, topic: &str, body: &Value) -> Value {
    let mut frame = json!({
        "bus_protocol": "ramflux.local_bus.v1",
        "frame_id": seq.map_or_else(|| "event_x".to_owned(), |s| format!("event_{s}")),
        "kind": "event", "request_id": "desk_stream", "account_id": account_field(account),
        "sdk_api": "stream", "method": topic, "body": body, "trace_id": null,
    });
    if let Some(seq) = seq {
        frame["seq"] = json!(seq);
        frame["at"] = json!(1_000 + seq);
    }
    frame
}

fn send(writer: &Writer, frame: &Value) -> bool {
    write_frame(&mut writer.lock().expect("writer"), frame).is_ok()
}

impl Journal {
    /// The daemon-level job journal (always on, no account).
    pub fn daemon() -> Self {
        Self { enabled: true, daemon: true, ..Self::default() }
    }

    /// Turns the stream on or off (off: `bus.subscribe` is refused as by a
    /// daemon without it).
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Answers a `bus.subscribe` frame that came on `connection`, writing
    /// on `writer`.
    pub fn subscribe(&mut self, request: &Value, connection: usize, writer: &Writer) -> Subscribed {
        if !self.enabled {
            return Subscribed::Refused(
                "ValidationFailed",
                "local bus error: unsupported local bus method: bus.subscribe".to_owned(),
            );
        }
        let account = if self.daemon {
            String::new()
        } else {
            let Some(account) = request["account_id"].as_str().map(str::to_owned) else {
                return Subscribed::Refused(
                    "ValidationFailed",
                    "local bus error: bus.subscribe requires account_id".to_owned(),
                );
            };
            account
        };
        let body = &request["body"];
        let topics: Vec<String> = body["topics"]
            .as_array()
            .map(|t| t.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default();
        let journal = self.accounts.entry(account.clone()).or_default();
        let (head, floor) = (journal.head, journal.floor());
        let cursor = body["since_seq"].as_u64().unwrap_or(head);
        if cursor < floor || cursor > head {
            return Subscribed::Refused(
                "resync_required",
                "local bus error: resync_required".to_owned(),
            );
        }
        let response = json!({
            "bus_protocol": "ramflux.local_bus.v1", "frame_id": "frame_resp",
            "kind": "response", "ok": true, "request_id": request["request_id"],
            "account_id": account_field(&account), "sdk_api": request["sdk_api"],
            "method": request["method"],
            "body": { "subscribed": true, "head_seq": head, "floor_seq": floor },
            "trace_id": null,
        });
        if !send(writer, &response) {
            return Subscribed::Following;
        }
        let mut delivered = cursor;
        for entry in journal.entries.iter().filter(|e| e.seq > cursor) {
            if topics.contains(&entry.topic) {
                if !send(writer, &event_frame(&account, Some(entry.seq), &entry.topic, &entry.body))
                {
                    return Subscribed::Following;
                }
                delivered = entry.seq;
            }
        }
        self.followers.push(Follower {
            connection,
            account,
            topics,
            writer: writer.clone(),
            delivered,
        });
        Subscribed::Following
    }

    /// Appends an entry for `account` and delivers it to its streams;
    /// returns its sequence number.
    pub fn publish(&mut self, account: &str, topic: &str, body: Value) -> u64 {
        let journal = self.accounts.entry(account.to_owned()).or_default();
        journal.head += 1;
        let seq = journal.head;
        journal.entries.push(Entry { seq, topic: topic.to_owned(), body: body.clone() });
        let frame = event_frame(account, Some(seq), topic, &body);
        self.followers.retain_mut(|f| {
            if f.account != account || !f.topics.iter().any(|t| t == topic) {
                return true;
            }
            f.delivered = seq;
            send(&f.writer, &frame)
        });
        seq
    }

    /// Writes a frame with any sequence number (or none) to `account`'s
    /// streams without journaling it: a duplicate or out-of-order delivery.
    pub fn deliver_raw(&mut self, account: &str, seq: Option<u64>, topic: &str, body: &Value) {
        let frame = event_frame(account, seq, topic, body);
        self.followers.retain(|f| f.account != account || send(&f.writer, &frame));
    }

    /// The daemon restarted: `account`'s entries are gone, its streams end
    /// (their connections close) and the next entry is numbered 1 again, so
    /// a cursor from before is refused with `resync_required`.
    pub fn restart(&mut self, account: &str) {
        self.end_streams(account, false);
        let journal = self.accounts.entry(account.to_owned()).or_default();
        journal.entries.clear();
        journal.head = 0;
    }

    /// Forgets `account`'s entries below `floor` (retention).
    pub fn prune_below(&mut self, account: &str, floor: u64) {
        let journal = self.accounts.entry(account.to_owned()).or_default();
        journal.entries.retain(|e| e.seq >= floor);
        journal.head = journal.head.max(floor.saturating_sub(1));
    }

    /// Ends `account`'s streams: with a final `stream.lagged` frame when
    /// `lagged`, then the connection closes.
    pub fn end_streams(&mut self, account: &str, lagged: bool) {
        self.followers.retain(|f| {
            if f.account != account {
                return true;
            }
            if lagged {
                let body = json!({ "last_delivered_seq": f.delivered });
                send(&f.writer, &event_frame(account, None, "stream.lagged", &body));
            }
            let _ = f.writer.lock().expect("writer").shutdown(std::net::Shutdown::Both);
            false
        });
    }

    /// The client closed `connection`: its stream ends.
    pub fn disconnect(&mut self, connection: usize) {
        self.followers.retain(|f| f.connection != connection);
    }

    /// Whether `account` has a stream open.
    pub fn streaming(&self, account: &str) -> bool {
        self.followers.iter().any(|f| f.account == account)
    }

    /// The newest entry of `account`.
    pub fn head(&self, account: &str) -> u64 {
        self.accounts.get(account).map_or(0, |a| a.head)
    }

    /// Drops every stream (the fake shuts down).
    pub fn clear(&mut self) {
        self.followers.clear();
    }
}
