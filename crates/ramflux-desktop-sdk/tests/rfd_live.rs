// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The SDK source against a real `rf daemon` from the pinned product
//! revision, without a node: daemon status, the empty node list, the
//! account list, error mapping, diagnostics, a stopped daemon that
//! disconnects, comes back and is re-snapshotted, many requests in flight on
//! one connection, the retry key read by the daemon, and a graceful stop.
//!
//! Ignored by default. Run with a pinned `rf`:
//!
//! ```sh
//! RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=/path/to/bin/rf \
//!   cargo test -p ramflux-desktop-sdk --test rfd_live -- --ignored --test-threads=1
//! ```
//!
//! A lock and unlock round trip needs an account, and creating, unlocking
//! or restoring one opens a gateway session on a node, so it is not
//! exercised without a node; unlocking an account the daemon does not know
//! is checked as an error instead. For the same reason contact calls are
//! checked for routing only. Since the account actors (product b38b4e4) the
//! daemon checks that the account is open before it looks at the method or
//! body, so without a node a contact frame (and even an unknown method)
//! only reaches the account router; the bodies are checked against the
//! SDK's request types in `request_shapes.rs`, `action_shapes.rs` and
//! `contacts.rs`. Of the client read model (product 3f8d726), the
//! Supervisor's `account.list` `account_details` is read live; the account
//! directories and the actions they unlock need an open account, so they
//! are checked for routing the same way.

mod common;

use common::{Fed, config};
use ramflux_desktop_core::{
    AccountId, DomainEvent, Effect, FailureCode, ListState, Listing, LockState, SecretInput,
    Source as _, SourceStatus, methods,
};
use ramflux_desktop_sdk::SdkHub;
use ramflux_desktop_test::live_daemon::{self, LiveDaemon};
use serde_json::json;

/// The daemon for one test, or `None` (with a note) when live tests are off.
fn daemon(tag: &str) -> Option<LiveDaemon> {
    if !live_daemon::enabled() {
        println!("skipped: set {}=1 to run against a real daemon", live_daemon::ENABLE_ENV);
        return None;
    }
    match LiveDaemon::start(tag) {
        Ok(daemon) => Some(daemon),
        Err(error) => panic!("cannot start the daemon: {error}"),
    }
}

fn hub(daemon: &LiveDaemon) -> SdkHub {
    SdkHub::start(ramflux_desktop_sdk::SdkConfig {
        deadlines: ramflux_desktop_sdk::Deadlines::standard(),
        ..config(&daemon.socket())
    })
    .expect("bridge")
}

#[test]
#[ignore = "needs a real rf daemon: RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=..."]
fn daemon_status_nodes_and_accounts_map_from_a_real_daemon() {
    let Some(daemon) = daemon("map") else { return };
    let hub = hub(&daemon);
    let mut entry = Fed::new(hub.daemon_source());
    entry.until("connected and listed", |s, _| {
        s.source_status() == SourceStatus::Connected
            && s.is_listed(Listing::Nodes)
            && s.is_listed(Listing::Accounts)
    });
    let info = entry.store.daemon().cloned().expect("daemon info");
    assert_eq!(info.socket_path, daemon.socket().to_string_lossy(), "the daemon's own socket");
    assert_eq!(info.open_accounts, 0);
    assert_eq!(entry.store.nodes().count(), 0, "a fresh daemon has no node");
    assert_eq!(
        ListState::of(entry.store.is_listed(Listing::Nodes), true),
        ListState::Empty,
        "an empty node list is an explicit empty state"
    );
    assert_eq!(entry.store.accounts().count(), 0, "no account is open");
    // The reference CLI agrees.
    let cli = daemon.rf(&["account", "list"]).expect("rf account list");
    let listed: serde_json::Value = serde_json::from_slice(&cli.stdout).expect("json");
    assert_eq!(listed["accounts"], json!([]));
    assert!(entry.store.rejections().is_empty(), "{:?}", entry.store.rejections());
    assert_eq!(entry.names()[0], "CapabilitiesUpdated");
}

#[test]
#[ignore = "needs a real rf daemon: RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=..."]
fn an_account_the_daemon_has_not_open_maps_to_locked_and_unlock_fails_cleanly() {
    let Some(daemon) = daemon("lock") else { return };
    let hub = hub(&daemon);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-none")));
    account.until("locked", |s, _| {
        s.account(&AccountId::new("acct-none")).is_some_and(|a| a.lock_state == LockState::Locked)
    });
    let mut entry = Fed::new(hub.daemon_source());
    entry.until("connected", |s, _| s.source_status() == SourceStatus::Connected);
    entry.source.submit(Effect::SecretCall {
        method: methods::ACCOUNT_UNLOCK.to_owned(),
        body: json!({ "account_id": "acct-none" }),
        secrets: vec![SecretInput::new("not a real passphrase")],
    });
    entry.until("unlock failed", |s, _| !s.failures().is_empty());
    let failure = entry.store.failures()[0].clone();
    println!("unlock of an unknown account: {failure:?}");
    assert_eq!(failure.method, methods::ACCOUNT_UNLOCK);
    assert!(
        matches!(failure.code, FailureCode::Daemon(_)),
        "a daemon refusal, not a transport error: {failure:?}"
    );
    assert!(!failure.message.contains("not a real passphrase"), "no secret in the message");
    assert_eq!(entry.store.source_status(), SourceStatus::Connected, "a refusal keeps the link");
}

#[test]
#[ignore = "needs a real rf daemon: RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=..."]
fn diagnostics_export_writes_a_redacted_bundle_from_the_real_daemon() {
    let Some(daemon) = daemon("diag") else { return };
    let hub = hub(&daemon);
    let mut entry = Fed::new(hub.daemon_source());
    entry.until("connected", |s, _| s.source_status() == SourceStatus::Connected);
    let path = daemon.dir().join("diagnostics.json");
    entry.source.submit(Effect::Call {
        method: methods::DIAGNOSTICS_EXPORT.to_owned(),
        body: json!({ "path": path.to_string_lossy() }),
    });
    entry.until("written", |_, _| path.exists());
    let bundle: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("bundle")).expect("json");
    assert_eq!(bundle["schema"], "ramflux.desktop.diagnostics.v1");
    assert_eq!(bundle["daemon"]["reachable"], true);
    assert_eq!(bundle["nodes"], json!([]));
}

#[test]
#[ignore = "needs a real rf daemon: RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=..."]
fn a_stopped_daemon_disconnects_and_a_restarted_one_is_resnapshotted() {
    let Some(mut daemon) = daemon("again") else { return };
    let hub = hub(&daemon);
    let mut entry = Fed::new(hub.daemon_source());
    entry.until("connected", |s, _| {
        s.source_status() == SourceStatus::Connected && s.is_listed(Listing::Nodes)
    });
    daemon.stop();
    assert!(!daemon.is_running());
    // The next poll finds the daemon gone.
    entry.until("disconnected", |s, _| s.source_status() == SourceStatus::Disconnected);
    let seq_at_loss = entry.store.last_seq();
    daemon.restart().expect("restart");
    entry.until("reconnected", |s, _| s.source_status() == SourceStatus::Connected);
    entry.until("re-snapshotted", |_, envelopes| {
        envelopes
            .iter()
            .rev()
            .take_while(|e| e.seq > seq_at_loss)
            .any(|e| matches!(&e.event, DomainEvent::DaemonInfoUpdated(_)))
    });
    assert!(entry.store.last_seq() > seq_at_loss, "the cursor keeps moving forward");
    assert_eq!(entry.source.last_seq(), entry.store.last_seq());
    assert!(entry.store.rejections().is_empty(), "{:?}", entry.store.rejections());
}

/// Plans a contact effect with a source that knows one link of each kind,
/// and returns the request the desktop would send.
fn contact_request(method: &str, participant: &str) -> ramflux_desktop_sdk::bus::BusRequest {
    use ramflux_desktop_core::{ContactLink, ContextId, ContextKind};
    use ramflux_desktop_sdk::plan::{Known, KnownLink, Plan, plan};
    struct Links(Vec<(&'static str, KnownLink)>);
    impl Known for Links {
        fn context_kind(&self, _: &ContextId) -> Option<ContextKind> {
            None
        }
        fn self_device(&self) -> Option<&str> {
            None
        }
        fn daemon_grant_id(&self, _: &str) -> Option<&str> {
            None
        }
        fn self_principal(&self) -> Option<&str> {
            Some("principal-me")
        }
        fn contact_link(&self, participant: &str) -> Option<&KnownLink> {
            self.0.iter().find(|(p, _)| *p == participant).map(|(_, l)| l)
        }
    }
    let link = |id: &str, requester: &str, target: &str, state| KnownLink {
        link_id: id.to_owned(),
        requester_id: requester.to_owned(),
        target_id: target.to_owned(),
        state,
    };
    let known = Links(vec![
        ("principal-ana", link("l1", "principal-me", "principal-ana", ContactLink::Accepted)),
        ("principal-sam", link("l2", "principal-sam", "principal-me", ContactLink::Incoming)),
    ]);
    let effect = Effect::Call {
        method: method.to_owned(),
        body: json!({ "participant_id": participant, "proposed_by": "Human", "confirmed_by": null }),
    };
    match plan(Some(&AccountId::new("acct-none")), effect, &known).plan {
        Plan::Call { request, .. } => request,
        other => panic!("{method}: expected a call, got {other:?}"),
    }
}

#[test]
#[ignore = "needs a real rf daemon: RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=..."]
fn every_contact_frame_reaches_the_account_router_of_the_real_daemon() {
    use ramflux_desktop_sdk::bus::BusError;
    use ramflux_desktop_sdk::bus::client::BusClient;
    let Some(daemon) = daemon("contacts") else { return };
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("rt");
    let client = BusClient::new(
        daemon.socket(),
        std::time::Duration::from_secs(5),
        ramflux_desktop_sdk::Deadlines::standard(),
    );
    for (method, participant) in [
        (methods::CONTACT_ADD, "principal-kai"),
        (methods::CONTACT_ACCEPT, "principal-sam"),
        (methods::CONTACT_REJECT, "principal-sam"),
        (methods::CONTACT_BLOCK, "principal-ana"),
        (methods::CONTACT_UNBLOCK, "principal-ana"),
        (methods::CONTACT_REMOVE, "principal-ana"),
    ] {
        let request = contact_request(method, participant);
        assert!(request.operation_id.is_some(), "{method} carries its retry key");
        let answer = runtime.block_on(client.call(&request));
        println!("{} {}: {answer:?}", request.method, request.body);
        assert!(
            matches!(&answer, Err(BusError::Daemon { message, .. })
                if message.contains("account not open: acct-none")),
            "{method}: {answer:?}"
        );
    }
    // The daemon still answers (no request left it in a bad state).
    let cli = daemon.rf(&["account", "list"]).expect("rf account list");
    assert!(cli.status.success());
}

#[test]
#[ignore = "needs a real rf daemon: RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=..."]
fn many_requests_run_concurrently_on_one_connection_of_a_real_daemon() {
    use ramflux_desktop_sdk::bus::BusRequest;
    use ramflux_desktop_sdk::bus::client::BusClient;
    let Some(daemon) = daemon("mux") else { return };
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("rt");
    let client = BusClient::new(
        daemon.socket(),
        std::time::Duration::from_secs(5),
        ramflux_desktop_sdk::Deadlines::standard(),
    );
    let answers = runtime.block_on(async {
        let mut set = tokio::task::JoinSet::new();
        for index in 0..48 {
            let client = client.clone();
            let (sdk_api, method) =
                [("daemon", "daemon.status"), ("node", "node.list"), ("account", "account.list")]
                    [index % 3];
            set.spawn(async move {
                client.call(&BusRequest::new(None, sdk_api, method, json!({}))).await
            });
        }
        let mut answers = Vec::new();
        while let Some(joined) = set.join_next().await {
            answers.push(joined.expect("task"));
        }
        answers
    });
    assert_eq!(answers.len(), 48);
    for answer in &answers {
        assert!(answer.is_ok(), "{answer:?}");
    }
    assert_eq!(client.connections_opened(), 1, "48 requests in flight on one connection");
}

#[test]
#[ignore = "needs a real rf daemon: RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=..."]
fn the_real_daemon_reads_the_retry_key_of_a_mutation() {
    use ramflux_desktop_sdk::bus::client::BusClient;
    use ramflux_desktop_sdk::bus::{BusError, BusRequest, idempotency};
    let Some(daemon) = daemon("keys") else { return };
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("rt");
    let client = BusClient::new(
        daemon.socket(),
        std::time::Duration::from_secs(5),
        ramflux_desktop_sdk::Deadlines::standard(),
    );
    let remove = |key: String| {
        BusRequest::new(None, "node", "node.remove", json!({ "node_id": "n-unknown" }))
            .with_operation_id(key)
    };
    // A key the daemon rejects proves it reads the field.
    let too_long = runtime.block_on(client.call(&remove("k".repeat(257))));
    println!("257-byte key: {too_long:?}");
    assert!(
        matches!(&too_long, Err(BusError::Daemon { message, .. }) if message.contains("operation_id")),
        "{too_long:?}"
    );
    // A desktop key is accepted; the request then fails on its own merits.
    let key = idempotency::fresh_key().expect("randomness");
    let first = runtime.block_on(client.call(&remove(key.clone())));
    println!("desktop key: {first:?}");
    assert!(
        matches!(&first, Err(BusError::Daemon { message, .. }) if !message.contains("operation_id")),
        "{first:?}"
    );
    // A failed operation stores no answer, and the daemon fails closed: the
    // same key is never applied again, it asks for reconciliation instead.
    let again = runtime.block_on(client.call(&remove(key)));
    println!("same key again: {again:?}");
    assert!(
        matches!(&again, Err(BusError::Daemon { message, .. })
            if message.contains("operation requires reconciliation")),
        "{again:?}"
    );
}

#[test]
#[ignore = "needs a real rf daemon: RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=..."]
fn the_hub_stops_a_real_daemon_gracefully() {
    let Some(mut daemon) = daemon("stop") else { return };
    let hub = hub(&daemon);
    let mut entry = Fed::new(hub.daemon_source());
    entry.until("connected", |s, _| s.source_status() == SourceStatus::Connected);
    let mut stop = hub.stop_daemon();
    let answer = stop.wait(std::time::Duration::from_secs(10));
    assert_eq!(answer, Some(Ok(())), "the daemon accepted the stop");
    assert!(daemon.wait_exit(std::time::Duration::from_secs(10)), "it exits by itself");
    let status = daemon.exit_status().expect("exited");
    println!("daemon exit status after daemon.stop: {status:?}");
    assert!(status.success(), "a clean exit, not a signal: {status:?}");
    assert!(!daemon.socket().exists(), "the daemon removed its socket");
    entry.until("disconnected", |s, _| s.source_status() == SourceStatus::Disconnected);
}

#[test]
#[ignore = "needs a real rf daemon: RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=..."]
fn the_real_account_list_carries_account_details_for_locked_accounts() {
    use ramflux_desktop_sdk::bus::BusRequest;
    use ramflux_desktop_sdk::bus::client::BusClient;
    let Some(daemon) = daemon("details") else { return };
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("rt");
    let client = BusClient::new(
        daemon.socket(),
        std::time::Duration::from_secs(5),
        ramflux_desktop_sdk::Deadlines::standard(),
    );
    let listed = runtime
        .block_on(client.call(&BusRequest::new(None, "account", "account.list", json!({}))))
        .expect("account.list");
    println!("account.list: {listed}");
    assert_eq!(
        listed["account_details"],
        json!([]),
        "the client read model field, empty on a fresh daemon"
    );
    assert_eq!(listed["accounts"], json!([]));
    let hub = hub(&daemon);
    let mut entry = Fed::new(hub.daemon_source());
    entry.until("accounts listed", |s, _| s.is_listed(Listing::Accounts));
    assert_eq!(entry.store.accounts().count(), 0);
    assert!(entry.store.rejections().is_empty(), "{:?}", entry.store.rejections());
}

/// The requests of the directories and of the actions they unlock, for an
/// account the daemon has not opened.
fn read_model_requests() -> Vec<ramflux_desktop_sdk::bus::BusRequest> {
    use ramflux_desktop_core::{ContextId, ContextKind};
    use ramflux_desktop_sdk::directory::{
        Conversation, DirectRoute, Directory, Identity, IdentityDevice,
    };
    use ramflux_desktop_sdk::plan::fetch::{Fetch, Paged};
    use ramflux_desktop_sdk::plan::{Known, Plan, plan};
    struct Listed(Directory);
    impl Known for Listed {
        fn context_kind(&self, _: &ContextId) -> Option<ContextKind> {
            Some(ContextKind::Direct)
        }
        fn self_device(&self) -> Option<&str> {
            Some("device-me")
        }
        fn daemon_grant_id(&self, _: &str) -> Option<&str> {
            None
        }
        fn directory(&self) -> Option<&Directory> {
            Some(&self.0)
        }
    }
    let mut directory =
        Directory { self_principal: Some("principal-me".to_owned()), ..Directory::default() };
    directory.conversations.insert(
        ContextId::new("dm-ana"),
        Conversation {
            kind: Some(ContextKind::Direct),
            title: None,
            group_id: Some("g-team".to_owned()),
            direct_route: Some(DirectRoute {
                commitment: Some("cm-ana".to_owned()),
                device_ids: vec!["device-ana".to_owned()],
                target: Some("target-ana".to_owned()),
                federation: None,
            }),
        },
    );
    directory.identities.insert(
        "principal-ana".to_owned(),
        Identity {
            commitment: Some("cm-ana".to_owned()),
            is_self: false,
            devices: vec![IdentityDevice {
                device_id: "device-ana".to_owned(),
                signing_key: Some("key-ana".to_owned()),
                target: Some("target-ana".to_owned()),
            }],
        },
    );
    directory.objects.insert("o-1".into(), ramflux_desktop_core::Direction::Outbound);
    let known = Listed(directory);
    let account = AccountId::new("acct-none");
    let mut requests: Vec<_> = [
        Paged::Identities,
        Paged::Conversations,
        Paged::Groups,
        Paged::Objects,
        Paged::Approvals,
        Paged::McpServers,
    ]
    .into_iter()
    .map(|paged| Fetch::first(paged).request(Some("acct-none")))
    .collect();
    for (method, body) in [
        (methods::SEND_MESSAGE, json!({ "context_id": "dm-ana", "text": "hi" })),
        (methods::SAFETY_VERIFY, json!({ "participant_id": "principal-ana" })),
        (methods::OBJECT_PULL, json!({ "object_id": "o-1" })),
        (methods::OBJECT_RETRY, json!({ "object_id": "o-1" })),
        (
            methods::GROUP_ADD_MEMBER,
            json!({ "context_id": "dm-ana", "participant_id": "principal-ana", "role": "Member" }),
        ),
        (
            methods::GROUP_CREATE_INVITE,
            json!({ "context_id": "dm-ana", "participant_id": "principal-ana", "role": "Member" }),
        ),
    ] {
        let effect = Effect::Call { method: method.to_owned(), body };
        match plan(Some(&account), effect, &known).plan {
            Plan::Call { request, .. } | Plan::Open { request, .. } => requests.push(request),
            other => panic!("{method}: expected one request, got {other:?}"),
        }
    }
    requests
}

#[test]
#[ignore = "needs a real rf daemon: RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=..."]
fn every_directory_and_action_frame_reaches_the_account_router_of_the_real_daemon() {
    use ramflux_desktop_sdk::bus::BusError;
    use ramflux_desktop_sdk::bus::client::BusClient;
    let Some(daemon) = daemon("readmodel") else { return };
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("rt");
    let client = BusClient::new(
        daemon.socket(),
        std::time::Duration::from_secs(5),
        ramflux_desktop_sdk::Deadlines::standard(),
    );
    let requests = read_model_requests();
    assert_eq!(requests.len(), 12);
    for request in &requests {
        let answer = runtime.block_on(client.call(request));
        println!("{} {}: {answer:?}", request.method, request.body);
        assert!(
            matches!(&answer, Err(BusError::Daemon { message, .. })
                if message.contains("account not open: acct-none")),
            "{}: {answer:?}",
            request.method
        );
    }
    let cli = daemon.rf(&["account", "list"]).expect("rf account list");
    assert!(cli.status.success(), "the daemon still answers");
}
