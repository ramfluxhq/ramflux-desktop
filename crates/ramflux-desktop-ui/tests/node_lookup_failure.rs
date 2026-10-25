// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A node lookup the daemon cannot complete is explained under the add-node
//! form: an unreachable host on macOS names the Local Network permission
//! and where to grant it; any other failure shows the daemon's reason. On
//! macOS a lookup blocked that way is repeated a moment later, which is
//! when the system has refreshed its record of the app (DESK-F19).

mod common;

use std::time::Duration;

use common::*;
use gpui_kit::TestAppContext;
use ramflux_desktop_core::{Listing, SourceStatus};
use ramflux_desktop_sdk::SdkConfig;
use ramflux_desktop_test::fake_bus::{FakeBus, Reply, socket_path};
use ramflux_desktop_ui::i18n::tr;
use ramflux_desktop_ui::shell::{ShellOptions, SourceChoice};
use serde_json::json;

fn options(path: &std::path::Path) -> ShellOptions {
    let config = SdkConfig {
        deadlines: ramflux_desktop_sdk::Deadlines::uniform(Duration::from_secs(3)),
        poll_fast: Duration::from_millis(80),
        poll_slow: Duration::from_millis(300),
        reconnect_backoff: Duration::from_millis(150),
        local_network_retry_ms: [40, 40],
        ..SdkConfig::new(path)
    };
    ShellOptions { source: SourceChoice::Rfd(config), ..ShellOptions::for_scenario("", true) }
}

fn wait(cx: &mut TestAppContext, what: &str, mut done: impl FnMut(&mut TestAppContext) -> bool) {
    let limit = std::time::Instant::now() + common::WAIT_LIMIT;
    while !done(cx) {
        assert!(std::time::Instant::now() < limit, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(10));
        advance(cx, 50);
    }
}

/// Looks a node up on a daemon that fails `node.add` with `message`, and
/// returns the label of the failure shown under the form.
fn lookup_failure(cx: &mut TestAppContext, name: &str, message: &'static str) -> Option<String> {
    let path = socket_path(name);
    let fake = FakeBus::start(&path, move |method, _, _| match method {
        "node.add" => Reply::Error("node", message.to_owned()),
        "daemon.status" => Reply::Ok(json!({ "accounts": 0, "socket_path": "fake" })),
        "node.list" => Reply::Ok(json!({ "default_node_id": null, "nodes": [] })),
        "account.list" => Reply::Ok(json!({ "active_account_id": null, "accounts": [] })),
        _ => Reply::Ok(json!({})),
    });
    let (handle, shell) = open_with(cx, options(&path));
    wait(cx, "the node step", |cx| {
        with_store(cx, &shell, |s| {
            s.source_status() == SourceStatus::Connected && s.is_listed(Listing::Nodes)
        })
    });
    assert!(!exists(cx, handle, "entry-node-add-failure"), "no failure before a lookup");
    type_into(cx, handle, "node-url", "https://node.local.invalid");
    click(cx, handle, "node-add");
    wait(cx, "the failure", |cx| exists(cx, handle, "entry-node-add-failure"));
    let label = in_window(cx, handle, |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window.find("entry-node-add-failure").label().map(str::to_owned)
    });
    fake.shutdown();
    label
}

#[gpui_kit::test]
fn an_unreachable_host_names_the_local_network_permission(cx: &mut TestAppContext) {
    let label = lookup_failure(
        cx,
        "lan-65",
        "node error: client record fetch failed: io error: HostUnreachable (os error 65)",
    );
    if cfg!(target_os = "macos") {
        let text = tr("backend.failure.local_network");
        assert_eq!(label.as_deref(), Some(text.as_ref()), "the translated permission hint");
        assert_ne!(text.as_ref(), "backend.failure.local_network", "a translation exists");
    } else {
        assert!(label.is_some_and(|l| l.contains("os error 65")), "the daemon's reason");
    }
}

#[gpui_kit::test]
fn a_failed_request_to_a_local_network_host_suggests_the_permission(cx: &mut TestAppContext) {
    let label = lookup_failure(
        cx,
        "lan-req",
        "local bus error: error sending request for url \
         (http://192.168.0.10:8080/.well-known/ramflux/client)",
    );
    if cfg!(target_os = "macos") {
        let text = ramflux_desktop_ui::i18n::backend_text(
            "backend.failure.local_network_maybe {\"host\":\"192.168.0.10\"}",
        );
        assert_eq!(label.as_deref(), Some(text.as_ref()));
        assert!(text.contains("192.168.0.10"), "the host is named: {text}");
    } else {
        assert!(label.is_some_and(|l| l.contains("error sending request")));
    }
}

#[gpui_kit::test]
fn any_other_lookup_failure_shows_the_daemons_reason(cx: &mut TestAppContext) {
    let label = lookup_failure(cx, "lan-sig", "client record verification failed");
    assert!(
        label.as_deref().is_some_and(|l| l.contains("client record verification failed")),
        "{label:?}"
    );
}

#[gpui_kit::test]
fn a_lookup_blocked_once_by_local_network_privacy_goes_through_on_the_repeat(
    cx: &mut TestAppContext,
) {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let url = "http://192.168.31.136:59482";
    let adds = Arc::new(AtomicUsize::new(0));
    let seen = adds.clone();
    let path = socket_path("lan-repeat");
    let fake = FakeBus::start(&path, move |method, _, _| match method {
        // macOS drops the first connection after an app update while it
        // refreshes its record of the app; the daemon reports the request.
        "node.add" if seen.fetch_add(1, Ordering::SeqCst) == 0 => Reply::Error(
            "node",
            format!(
                "local bus error: error sending request for url ({url}/.well-known/ramflux/client)"
            ),
        ),
        "node.add" => Reply::Ok(json!({ "node": {
            "node_id": "node-lan", "discovery_url": url, "node_public_key": "key",
            "pin_source": "tofu",
            "gateway": { "quic_addr": "192.168.31.136:7443", "tcp_addr": "", "server_name": "lan" },
            "relay": { "quic_addr": "192.168.31.136:7543", "server_name": "lan" },
            "prekey_http_url": "", "signaling_turn_addr": "", "protocol_versions": ["ramflux.v1"],
            "client_capabilities": ["gateway", "relay", "prekey"],
            "discovered_at": 1_790_000_000_u64, "expires_at": 1_890_000_000_u64,
            "enabled": true, "builtin": false, "safety_warnings": ["tofu_unverified"],
        }, "persisted": false })),
        "daemon.status" => Reply::Ok(json!({ "accounts": 0, "socket_path": "fake" })),
        "node.list" => Reply::Ok(json!({ "default_node_id": null, "nodes": [] })),
        "account.list" => Reply::Ok(json!({ "active_account_id": null, "accounts": [] })),
        _ => Reply::Ok(json!({})),
    });
    let (handle, shell) = open_with(cx, options(&path));
    wait(cx, "the node step", |cx| {
        with_store(cx, &shell, |s| {
            s.source_status() == SourceStatus::Connected && s.is_listed(Listing::Nodes)
        })
    });
    type_into(cx, handle, "node-url", url);
    click(cx, handle, "node-add");
    if cfg!(target_os = "macos") {
        wait(cx, "the node found on the repeat", |cx| {
            with_store(cx, &shell, |s| s.nodes().any(|n| n.id.to_string() == "node-lan"))
        });
        assert_eq!(adds.load(Ordering::SeqCst), 2, "one repeat");
        assert!(!exists(cx, handle, "entry-node-add-failure"), "no failure shown");
    } else {
        wait(cx, "the failure", |cx| exists(cx, handle, "entry-node-add-failure"));
        assert_eq!(adds.load(Ordering::SeqCst), 1, "no permission to wait for elsewhere");
    }
    fake.shutdown();
}
