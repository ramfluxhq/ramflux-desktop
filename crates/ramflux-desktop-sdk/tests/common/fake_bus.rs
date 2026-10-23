// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A fake local-bus daemon on a Unix socket, speaking the daemon's framing
//! (4-byte big-endian length, then a JSON frame), for adapter tests.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

/// How the fake answers one request.
#[derive(Clone, Debug)]
pub enum Reply {
    /// A response frame with this body.
    Ok(Value),
    /// An error frame.
    Error(&'static str, String),
    /// Wait, then answer.
    Slow(Duration, Value),
}

type Handler = dyn Fn(&str, Option<&str>, &Value) -> Reply + Send + Sync;

/// One request the fake received.
#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    pub account: Option<String>,
    pub body: Value,
}

pub struct FakeBus {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    seen: Arc<Mutex<Vec<Seen>>>,
    subscribers: Arc<Mutex<Vec<(String, UnixStream)>>>,
    streams: Arc<Mutex<Vec<UnixStream>>>,
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
        let (s, h, sn, sub, st) =
            (stop.clone(), handler, seen.clone(), subscribers.clone(), streams.clone());
        thread::spawn(move || {
            while !s.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(false).ok();
                        if let Ok(clone) = stream.try_clone() {
                            st.lock().expect("streams").push(clone);
                        }
                        let (h, sn, sub) = (h.clone(), sn.clone(), sub.clone());
                        thread::spawn(move || serve(stream, &*h, &sn, &sub));
                    }
                    Err(_) => thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        Self { path: path.to_path_buf(), stop, seen, subscribers, streams }
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

fn write_frame(stream: &mut UnixStream, frame: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(frame).expect("frame json");
    let len = u32::try_from(body.len()).expect("frame size");
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&body)
}

fn serve(
    mut stream: UnixStream,
    handler: &Handler,
    seen: &Mutex<Vec<Seen>>,
    subscribers: &Mutex<Vec<(String, UnixStream)>>,
) {
    while let Some(request) = read_frame(&mut stream) {
        let method = request["method"].as_str().unwrap_or_default().to_owned();
        let account = request["account_id"].as_str().map(str::to_owned);
        let body = request["body"].clone();
        seen.lock().expect("seen").push(Seen {
            method: method.clone(),
            account: account.clone(),
            body: body.clone(),
        });
        let reply = if method == "subscription.open" {
            if let (Some(account), Ok(clone)) = (&account, stream.try_clone()) {
                subscribers.lock().expect("subs").push((account.clone(), clone));
            }
            Reply::Ok(json!({ "subscribed": true }))
        } else {
            handler(&method, account.as_deref(), &body)
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
        if write_frame(&mut stream, &frame).is_err() {
            break;
        }
    }
}

/// Recorded and SDK-shaped daemon answers, by method.
pub fn frames() -> BTreeMap<String, Value> {
    let text = include_str!("../fixtures/rfd_frames.json");
    let all: BTreeMap<String, Value> = serde_json::from_str(text).expect("fixture json");
    all.into_iter().filter(|(k, _)| !k.starts_with('_')).collect()
}

/// A unique socket path in the temp dir (kept short for the Unix limit).
pub fn socket_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rfdt-{name}-{}.sock", std::process::id()))
}
