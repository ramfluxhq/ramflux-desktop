// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Optional bounded metadata from actual request sends, shared by all connections.
//! No account ids, request ids, operation keys, bodies or answers are retained.

use ramflux_sdk::LocalBusFrame;
use serde::Serialize;
use std::path::Path;

/// Maximum retained send records across all connections.
pub const CAPACITY: usize = 256;

/// Public metadata for one send attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    /// Milliseconds since tracing initialized, from a monotonic clock.
    pub monotonic_ms: u64,
    /// Daemon method name.
    pub method: String,
    /// Wire frame kind.
    pub kind: String,
    /// Whether the request carries an operation key, never the key itself.
    pub keyed: bool,
    /// Ordinary, stream, push, job or attended connection.
    pub connection_class: String,
    /// Data, subscription, setup or control request.
    pub frame_class: String,
}

/// Records metadata immediately before handing a frame to the wire client.
#[inline]
pub(crate) fn record(socket: &Path, connection: &str, frame: &LocalBusFrame) {
    #[cfg(feature = "bus-trace")]
    enabled::record(socket, connection, frame);
    #[cfg(not(feature = "bus-trace"))]
    let _ = (socket, connection, frame);
}

/// New records for one daemon endpoint, identified by an opaque sequence.
/// Returns no entries unless `bus-trace` is enabled.
pub fn after(socket: &Path, sequence: u64) -> Vec<(u64, Entry)> {
    #[cfg(feature = "bus-trace")]
    return enabled::after(socket, sequence);
    #[cfg(not(feature = "bus-trace"))]
    {
        let _ = (socket, sequence);
        Vec::new()
    }
}

#[cfg(feature = "bus-trace")]
mod enabled {
    use super::*;
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;

    struct Ring {
        start: Instant,
        sequence: u64,
        entries: VecDeque<(u64, PathBuf, Entry)>,
    }

    impl Ring {
        fn append(&mut self, socket: &Path, entry: Entry) {
            if self.entries.len() == CAPACITY {
                self.entries.pop_front();
            }
            self.sequence += 1;
            self.entries.push_back((self.sequence, socket.to_owned(), entry));
        }

        #[cfg(test)]
        fn export(&self, path: &Path) {
            let entries: Vec<_> = self.entries.iter().map(|(_, _, e)| e).collect();
            if let Ok(bytes) = serde_json::to_vec(&entries) {
                let _ = std::fs::write(path, bytes);
            }
        }
    }

    fn ring() -> &'static Mutex<Ring> {
        static RING: OnceLock<Mutex<Ring>> = OnceLock::new();
        RING.get_or_init(|| {
            Mutex::new(Ring { start: Instant::now(), sequence: 0, entries: VecDeque::new() })
        })
    }

    pub(super) fn record(socket: &Path, connection: &str, frame: &LocalBusFrame) {
        let Ok(mut ring) = ring().lock() else { return };
        let frame_class = match frame.method.as_str() {
            "subscription.open"
                if frame
                    .body
                    .get("topics")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(Vec::is_empty) =>
            {
                "setup"
            }
            "bus.subscribe" | "subscription.open" => "subscription",
            "bus.unsubscribe" | "subscription.close" | "daemon.stop" => "control",
            _ => "data",
        };
        let entry = Entry {
            monotonic_ms: u64::try_from(ring.start.elapsed().as_millis()).unwrap_or(u64::MAX),
            method: frame.method.clone(),
            kind: format!("{:?}", frame.kind),
            keyed: frame.operation_id.is_some(),
            connection_class: connection.into(),
            frame_class: frame_class.into(),
        };
        ring.append(socket, entry);
        if let Some(path) = std::env::var_os("RAMFLUX_BUS_TRACE_FILE") {
            // The file is a bounded snapshot, with exactly the public metadata.
            let entries: Vec<_> = ring.entries.iter().map(|(_, _, e)| e.clone()).collect();
            drop(ring);
            // Serialization and file I/O never hold the global send ring lock.
            if let Ok(bytes) = serde_json::to_vec(&entries) {
                let _ = std::fs::write(path, bytes);
            }
        }
    }

    #[cfg(test)]
    pub(super) fn check_retention_and_export() {
        let mut ring = Ring { start: Instant::now(), sequence: 0, entries: VecDeque::new() };
        for n in 0..300 {
            ring.append(
                Path::new("private-endpoint"),
                Entry {
                    monotonic_ms: n,
                    method: "daemon.status".into(),
                    kind: "Request".into(),
                    keyed: false,
                    connection_class: "ordinary".into(),
                    frame_class: "data".into(),
                },
            );
        }
        assert_eq!(ring.entries.len(), CAPACITY);
        assert_eq!(ring.entries.front().expect("oldest").2.monotonic_ms, 44);
        let path = std::env::temp_dir().join(format!("desktop-trace-{}.json", std::process::id()));
        ring.export(&path);
        let bytes = std::fs::read(&path).expect("export file");
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("export JSON");
        assert_eq!(json.as_array().expect("ring").len(), CAPACITY);
        assert!(
            json.as_array()
                .expect("ring")
                .iter()
                .all(|e| e.as_object().expect("metadata").len() == 6)
        );
        assert!(!String::from_utf8(bytes).expect("UTF8").contains("private-endpoint"));
        std::fs::remove_file(path).expect("cleanup fixture");
    }

    pub(super) fn after(socket: &Path, sequence: u64) -> Vec<(u64, Entry)> {
        let Ok(ring) = ring().lock() else { return Vec::new() };
        ring.entries
            .iter()
            .filter(|(seq, endpoint, _)| *seq > sequence && endpoint == socket)
            .map(|(seq, _, entry)| (*seq, entry.clone()))
            .collect()
    }
}

#[cfg(all(test, feature = "bus-trace"))]
mod tests;

#[cfg(all(test, feature = "bus-trace"))]
mod retention_tests {
    use super::*;

    #[test]
    fn ring_export_is_bounded_and_contains_only_send_metadata() {
        enabled::check_retention_and_export();
    }
}
