// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The redacted diagnostics bundle written from live daemon reads.
//!
//! It carries daemon reachability, node ids with their endpoints and pin
//! state, and counts. It never carries keys, certificates, account or
//! principal ids, or message content.

use serde_json::{Value, json};

use crate::translate::{list, text};

/// The bundle text from `daemon.status` and `node.list` answers.
pub fn bundle(status: &Value, nodes: &Value) -> String {
    let nodes: Vec<Value> = list(nodes, "nodes")
        .iter()
        .map(|n| {
            json!({
                "node_id": text(n, "node_id"),
                "pin_source": text(n, "pin_source"),
                "enabled": n["enabled"],
                "gateway": n["gateway"]["quic_addr"],
                "relay": n["relay"]["quic_addr"],
                "safety_warnings": n["safety_warnings"],
            })
        })
        .collect();
    let bundle = json!({
        "schema": "ramflux.desktop.diagnostics.v1",
        "daemon": { "reachable": true, "open_accounts": status["accounts"] },
        "nodes": nodes,
    });
    serde_json::to_string_pretty(&bundle).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_drops_keys_and_certificates() {
        let nodes = json!({ "nodes": [{
            "node_id": "n-1", "pin_source": "tofu", "enabled": true,
            "node_public_key": "SECRETKEY", "ca_cert_pem": "-----BEGIN",
            "gateway": { "quic_addr": "127.0.0.1:7443" }, "relay": { "quic_addr": "r:1" },
        }]});
        let text = bundle(&json!({ "accounts": 2, "socket_path": "/tmp/x" }), &nodes);
        assert!(text.contains("n-1") && text.contains("\"open_accounts\": 2"));
        assert!(!text.contains("SECRETKEY") && !text.contains("BEGIN"));
    }
}
