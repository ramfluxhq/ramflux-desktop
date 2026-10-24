// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A fake local-bus daemon on a Unix socket, speaking the daemon's framing
//! (4-byte big-endian length, then a JSON frame), for adapter and UI tests.
//!
//! Like the real daemon it answers the requests of one connection
//! concurrently (each answer carries its request id, so a slow answer does
//! not hold up the others), and it keeps retry receipts: a request whose
//! `operation_id` was already applied is answered from the receipt without
//! calling the handler again.
//!
//! It also keeps an event journal per account and serves `bus.subscribe`
//! from it ([`crate::fake_journal`]); the stream is off until a test turns
//! it on ([`FakeBus::enable_stream`]), so by default the fake answers as a
//! daemon without the stream and the source polls.
//!
//! Std only (a thread per connection and per request); behind the
//! `fake-bus` feature.

use std::collections::BTreeMap;
use std::io::Read;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

use crate::fake_journal::{Journal, Subscribed, Writer, write_frame};

/// How the fake answers one request.
#[derive(Clone, Debug)]
pub enum Reply {
    /// A response frame with this body.
    Ok(Value),
    /// An error frame.
    Error(&'static str, String),
    /// Wait, then answer.
    Slow(Duration, Value),
    /// Apply (the receipt records this answer), then close the connection
    /// without answering: the answer is lost on the way back.
    Lost(Value),
}

type Handler = dyn Fn(&str, Option<&str>, &Value) -> Reply + Send + Sync;

/// Answers recorded by retry key.
type Receipts = Mutex<BTreeMap<String, Value>>;

/// One request the fake received.
#[derive(Clone, Debug)]
pub struct Seen {
    /// Method.
    pub method: String,
    /// Account of the frame.
    pub account: Option<String>,
    /// Body.
    pub body: Value,
    /// The frame's request id.
    pub request_id: String,
    /// The connection it came on (numbered from 1 in accept order).
    pub connection: usize,
    /// The frame's absolute deadline, in Unix milliseconds.
    pub deadline_ms: Option<u64>,
    /// The frame's retry key.
    pub operation_id: Option<String>,
    /// Whether it was answered from a receipt (not applied again).
    pub replayed: bool,
}

/// Appends events to a fake daemon's journal (see [`FakeBus::publisher`]).
#[derive(Clone)]
pub struct Publisher {
    journal: Arc<Mutex<Journal>>,
}

impl Publisher {
    /// Appends an event to `account`'s journal and delivers it; returns its
    /// sequence number.
    pub fn publish(&self, account: &str, topic: &str, body: Value) -> u64 {
        self.journal.lock().expect("journal").publish(account, topic, body)
    }
}

/// The fake daemon. Dropping it does not stop it; call [`FakeBus::shutdown`].
pub struct FakeBus {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    seen: Arc<Mutex<Vec<Seen>>>,
    subscribers: Arc<Mutex<Vec<(String, UnixStream)>>>,
    streams: Arc<Mutex<Vec<UnixStream>>>,
    applied: Arc<AtomicUsize>,
    journal: Arc<Mutex<Journal>>,
}

/// What every connection of one fake shares.
struct Shared {
    handler: Arc<Handler>,
    seen: Arc<Mutex<Vec<Seen>>>,
    subscribers: Arc<Mutex<Vec<(String, UnixStream)>>>,
    receipts: Arc<Receipts>,
    applied: Arc<AtomicUsize>,
    journal: Arc<Mutex<Journal>>,
}

impl FakeBus {
    /// Serves `handler` at `path` (replacing any socket file there).
    pub fn start(
        path: &Path,
        handler: impl Fn(&str, Option<&str>, &Value) -> Reply + Send + Sync + 'static,
    ) -> Self {
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path).expect("bind fake bus");
        listener.set_nonblocking(true).expect("nonblocking");
        let handler: Arc<Handler> = Arc::new(handler);
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let subscribers = Arc::new(Mutex::new(Vec::new()));
        let streams = Arc::new(Mutex::new(Vec::new()));
        let applied = Arc::new(AtomicUsize::new(0));
        let journal = Arc::new(Mutex::new(Journal::default()));
        let shared = Arc::new(Shared {
            handler,
            seen: seen.clone(),
            subscribers: subscribers.clone(),
            receipts: Arc::new(Mutex::new(BTreeMap::new())),
            applied: applied.clone(),
            journal: journal.clone(),
        });
        let (s, st) = (stop.clone(), streams.clone());
        thread::spawn(move || {
            let mut connections = 0;
            while !s.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        connections += 1;
                        stream.set_nonblocking(false).ok();
                        if let Ok(clone) = stream.try_clone() {
                            st.lock().expect("streams").push(clone);
                        }
                        let shared = shared.clone();
                        thread::spawn(move || serve(stream, connections, &shared));
                    }
                    Err(_) => thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        Self { path: path.to_path_buf(), stop, seen, subscribers, streams, applied, journal }
    }

    fn journal(&self) -> std::sync::MutexGuard<'_, Journal> {
        self.journal.lock().expect("journal")
    }

    /// Serves `bus.subscribe` from the journal (a daemon with the stream).
    pub fn enable_stream(&self) {
        self.journal().set_enabled(true);
    }

    /// Refuses `bus.subscribe` again (as a daemon without the stream);
    /// open streams keep running until ended.
    pub fn disable_stream(&self) {
        self.journal().set_enabled(false);
    }

    /// Appends an event to `account`'s journal and delivers it to its
    /// streams; returns its sequence number.
    pub fn publish(&self, account: &str, topic: &str, body: Value) -> u64 {
        self.journal().publish(account, topic, body)
    }

    /// A handle that appends to the journal from elsewhere (a fake domain
    /// answering a request announces what it changed, as the daemon does).
    pub fn publisher(&self) -> Publisher {
        Publisher { journal: self.journal.clone() }
    }

    /// Writes an event with any sequence number to `account`'s streams
    /// without journaling it (a duplicate or out-of-order delivery).
    pub fn deliver_raw(&self, account: &str, seq: Option<u64>, topic: &str, body: &Value) {
        self.journal().deliver_raw(account, seq, topic, body);
    }

    /// Forgets `account`'s journal entries below `floor`.
    pub fn prune_below(&self, account: &str, floor: u64) {
        self.journal().prune_below(account, floor);
    }

    /// Ends `account`'s streams, with a final `stream.lagged` when `lagged`.
    pub fn end_streams(&self, account: &str, lagged: bool) {
        self.journal().end_streams(account, lagged);
    }

    /// Whether `account` has a stream open.
    pub fn streaming(&self, account: &str) -> bool {
        self.journal().streaming(account)
    }

    /// The newest journal entry of `account`.
    pub fn head(&self, account: &str) -> u64 {
        self.journal().head(account)
    }

    /// How many requests the handler applied (receipt answers excluded).
    pub fn applied(&self) -> usize {
        self.applied.load(Ordering::SeqCst)
    }

    /// Every request received so far.
    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().expect("seen").clone()
    }

    /// Pushes an event frame to every subscriber of `account`.
    pub fn push(&self, account: &str, topic: &str, body: Value) {
        let frame = json!({
            "bus_protocol": "ramflux.local_bus.v1", "frame_id": "frame_evt", "kind": "event",
            "request_id": "evt", "account_id": account, "sdk_api": "mcp", "method": topic,
            "body": body, "trace_id": null,
        });
        for (who, stream) in self.subscribers.lock().expect("subs").iter_mut() {
            if who == account {
                let _ = write_frame(stream, &frame);
            }
        }
    }

    /// Whether `account` has a subscription open.
    pub fn subscribed(&self, account: &str) -> bool {
        self.subscribers.lock().expect("subs").iter().any(|(who, _)| who == account)
    }

    /// Stops accepting, closes every connection and removes the socket.
    pub fn shutdown(self) {
        self.stop.store(true, Ordering::SeqCst);
        for stream in self.streams.lock().expect("streams").drain(..) {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        self.subscribers.lock().expect("subs").clear();
        self.journal().clear();
        let _ = std::fs::remove_file(&self.path);
        thread::sleep(Duration::from_millis(30));
    }
}

fn read_frame(stream: &mut UnixStream) -> Option<Value> {
    let mut len = [0_u8; 4];
    stream.read_exact(&mut len).ok()?;
    let mut body = vec![0_u8; u32::from_be_bytes(len) as usize];
    stream.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

fn serve(mut stream: UnixStream, connection: usize, shared: &Arc<Shared>) {
    let Ok(writer) = stream.try_clone() else { return };
    let writer: Writer = Arc::new(Mutex::new(writer));
    while let Some(request) = read_frame(&mut stream) {
        let (shared, writer) = (shared.clone(), writer.clone());
        let Ok(subscriber) = stream.try_clone() else { break };
        thread::spawn(move || answer(&request, connection, &shared, &writer, subscriber));
    }
    shared.journal.lock().expect("journal").disconnect(connection);
}

fn text(frame: &Value, field: &str) -> Option<String> {
    frame[field].as_str().map(str::to_owned)
}

fn answer(
    request: &Value,
    connection: usize,
    shared: &Shared,
    writer: &Writer,
    subscriber: UnixStream,
) {
    let method = text(request, "method").unwrap_or_default();
    let account = text(request, "account_id");
    let body = request["body"].clone();
    let operation_id = text(request, "operation_id");
    let receipt = operation_id
        .as_ref()
        .and_then(|key| shared.receipts.lock().expect("receipts").get(key).cloned());
    shared.seen.lock().expect("seen").push(Seen {
        method: method.clone(),
        account: account.clone(),
        body: body.clone(),
        request_id: text(request, "request_id").unwrap_or_default(),
        connection,
        deadline_ms: request["deadline_ms"].as_u64(),
        operation_id: operation_id.clone(),
        replayed: receipt.is_some(),
    });
    let reply = if let Some(stored) = receipt {
        Reply::Ok(stored)
    } else if method == "bus.subscribe" {
        match shared.journal.lock().expect("journal").subscribe(request, connection, writer) {
            Subscribed::Following => return,
            Subscribed::Refused(code, message) => Reply::Error(code, message),
        }
    } else if method == "subscription.open" {
        if let Some(account) = &account {
            shared.subscribers.lock().expect("subs").push((account.clone(), subscriber));
        }
        Reply::Ok(json!({ "subscribed": true }))
    } else {
        shared.applied.fetch_add(1, Ordering::SeqCst);
        let reply = (shared.handler)(&method, account.as_deref(), &body);
        if let (Some(key), Reply::Ok(answer) | Reply::Slow(_, answer) | Reply::Lost(answer)) =
            (&operation_id, &reply)
        {
            shared.receipts.lock().expect("receipts").insert(key.clone(), answer.clone());
        }
        reply
    };
    let mut frame = json!({
        "bus_protocol": "ramflux.local_bus.v1", "frame_id": "frame_resp",
        "request_id": request["request_id"], "account_id": request["account_id"],
        "sdk_api": request["sdk_api"], "method": method, "trace_id": null,
    });
    let map = frame.as_object_mut().expect("object");
    match reply {
        Reply::Ok(body) => {
            map.insert("kind".into(), json!("response"));
            map.insert("ok".into(), json!(true));
            map.insert("body".into(), body);
        }
        Reply::Slow(wait, body) => {
            thread::sleep(wait);
            map.insert("kind".into(), json!("response"));
            map.insert("ok".into(), json!(true));
            map.insert("body".into(), body);
        }
        Reply::Lost(_) => {
            let stream = writer.lock().expect("writer");
            let _ = stream.shutdown(std::net::Shutdown::Both);
            return;
        }
        Reply::Error(code, message) => {
            map.insert("kind".into(), json!("error"));
            map.insert("ok".into(), json!(false));
            map.insert("body".into(), Value::Null);
            map.insert(
                "error".into(),
                json!({ "code": code, "message": message, "retry_after_ms": null }),
            );
        }
    }
    let _ = write_frame(&mut writer.lock().expect("writer"), &frame);
}

/// A unique socket path in the temp dir (kept short for the Unix limit).
pub fn socket_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rfdt-{name}-{}.sock", std::process::id()))
}
