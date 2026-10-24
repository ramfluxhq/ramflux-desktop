// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A pass-through socket between one desktop and its daemon that writes a
//! line per local-bus frame: which way it went, its kind, method, sequence
//! number and, for `bus.subscribe` only, the cursor asked for and the
//! daemon's answer. No other body is written (an unlock carries the
//! passphrase), so the log is evidence of how the desktop followed the
//! stream: whether it subscribed with `since_seq` after a restart, which
//! events the daemon replayed, and that nothing was polled.
//!
//! Frames are a 4-byte big-endian length and a JSON body, both ways. The tap
//! forwards bytes unchanged; a frame it cannot parse is forwarded and noted.

use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// A listening tap; its socket is removed on drop (the accept thread ends
/// with the process).
pub struct Tap {
    socket: PathBuf,
}

impl Tap {
    /// Listens on `socket` and forwards every connection to `daemon`,
    /// logging frames to `log`.
    ///
    /// # Errors
    ///
    /// When the socket or the log cannot be made.
    pub fn start(socket: &Path, daemon: &Path, log: &Path) -> Result<Self, String> {
        let _ = std::fs::remove_file(socket);
        let listener =
            UnixListener::bind(socket).map_err(|e| format!("tap {}: {e}", socket.display()))?;
        let file = File::options()
            .create(true)
            .append(true)
            .open(log)
            .map_err(|e| format!("tap log {}: {e}", log.display()))?;
        let out = Arc::new(Mutex::new(file));
        let daemon = daemon.to_path_buf();
        std::thread::spawn(move || {
            for (n, client) in listener.incoming().enumerate() {
                let Ok(client) = client else { continue };
                let (daemon, out) = (daemon.clone(), Arc::clone(&out));
                std::thread::spawn(move || connect(n, client, &daemon, &out));
            }
        });
        Ok(Self { socket: socket.to_path_buf() })
    }

    /// The socket the desktop attaches to.
    pub fn socket(&self) -> &Path {
        &self.socket
    }
}

impl Drop for Tap {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
    }
}

fn connect(n: usize, client: UnixStream, daemon: &Path, out: &Arc<Mutex<File>>) {
    let Ok(server) = UnixStream::connect(daemon) else {
        write(out, &json!({ "conn": n, "note": "daemon socket refused" }));
        return;
    };
    let (Ok(client_rx), Ok(server_tx)) = (client.try_clone(), server.try_clone()) else { return };
    let up_out = Arc::clone(out);
    let up = std::thread::spawn(move || pump(n, "up", client_rx, server_tx, &up_out));
    pump(n, "down", server, client, out);
    let _ = up.join();
}

/// Copies frames from `from` to `to` until either side closes.
fn pump(n: usize, dir: &str, mut from: UnixStream, mut to: UnixStream, out: &Arc<Mutex<File>>) {
    loop {
        let mut len = [0_u8; 4];
        if from.read_exact(&mut len).is_err() {
            break;
        }
        let size = u32::from_be_bytes(len) as usize;
        let mut body = vec![0_u8; size];
        if from.read_exact(&mut body).is_err() || to.write_all(&len).is_err() {
            break;
        }
        if to.write_all(&body).is_err() {
            break;
        }
        write(out, &summary(n, dir, &body));
    }
    let _ = to.shutdown(std::net::Shutdown::Both);
    write(out, &json!({ "conn": n, "dir": dir, "note": "closed" }));
}

/// The line written for one frame: no bodies except a subscription's
/// cursor fields.
fn summary(n: usize, dir: &str, body: &[u8]) -> Value {
    let Ok(frame) = serde_json::from_slice::<Value>(body) else {
        return json!({ "conn": n, "dir": dir, "note": "unparsed frame", "bytes": body.len() });
    };
    let method = frame["method"].as_str().unwrap_or_default();
    let mut line = json!({
        "conn": n,
        "dir": dir,
        "kind": frame["kind"],
        "method": method,
        "request_id": frame["request_id"],
    });
    if !frame["seq"].is_null() {
        line["seq"] = frame["seq"].clone();
    }
    if method == "bus.subscribe" {
        for field in ["since_seq", "subscribed", "head_seq", "floor_seq"] {
            if !frame["body"][field].is_null() {
                line[field] = frame["body"][field].clone();
            }
        }
    }
    if method == "identity.directory" && dir == "down" {
        // Public directory data only: who is listed and with how many
        // devices (evidence for routes the desktop can or cannot use).
        let rows = frame["body"]["identities"].as_array().cloned().unwrap_or_default();
        line["identities"] = rows
            .iter()
            .map(|r| {
                json!({
                    "principal_id": r["principal_id"],
                    "is_self": r["is_self"],
                    "has_commitment": !r["contact_identity_commitment"].is_null(),
                    "devices": r["devices"].as_array().map_or(0, Vec::len),
                })
            })
            .collect();
    }
    if frame["kind"] == "error" {
        // The daemon's error frame: code, typed reason and message (never
        // secret material; the message is the daemon's own text).
        let error = &frame["error"];
        line["code"] = error["code"].clone();
        line["reason_code"] = error["reason_code"].clone();
        line["message"] = error["message"].clone();
    }
    if dir == "down" && matches!(method, "message.list" | "message.receive" | "conversation.start")
    {
        // Counts only: how many rows and whether their text was readable.
        let rows = frame["body"]["messages"].as_array().cloned().unwrap_or_default();
        line["messages"] = json!(rows.len());
        line["with_text"] = json!(
            rows.iter().filter(|r| r["body_utf8"].is_string() || r["text"].is_string()).count()
        );
        // Ids only (no text): which id each stored row carries.
        line["message_ids"] = rows.iter().map(|r| r["message_id"].clone()).collect();
        line["sender_ids"] = rows.iter().map(|r| r["sender_id"].clone()).collect();
    }
    if method == "message.submit" {
        // Ids only: the ids the desktop chose and the ones the daemon answers.
        let body = &frame["body"];
        line["message_id"] = body["message_id"].clone();
        line["envelope_id"] = body["envelope_id"].clone();
    }
    line
}

fn write(out: &Arc<Mutex<File>>, line: &Value) {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let mut line = line.clone();
    line["at_ms"] = json!(ms);
    if let Ok(mut file) = out.lock() {
        let _ = writeln!(file, "{line}");
    }
}

/// The tap's log lines.
pub fn read_log(log: &Path) -> Vec<Value> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}
