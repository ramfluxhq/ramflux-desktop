// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! One fake daemon with one unlocked account, answering from the SDK's
//! recorded frames plus a few contacts to start conversations with, and
//! recording what the desktop submits:
//!
//! - `principal-bo`: accepted, verified, routable, no direct conversation;
//! - `principal-cy`: accepted, key changed (`verification_stale`);
//! - `principal-dee`: accepted, a device with a target but no verified key;
//! - `principal-zed`: accepted, not listed by the identity directory.
//!
//! A successful `message.submit` adds its conversation (and attachment) to
//! the directories, as the daemon records them; `fail_submit` makes it fail.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::TestAppContext;
use ramflux_desktop_sdk::SdkConfig;
use ramflux_desktop_test::fake_bus::{FakeBus, Reply, socket_path};
use ramflux_desktop_ui::shell::{ShellOptions, SourceChoice};
use ramflux_desktop_ui::workspace::PrefsStore;
use serde_json::{Value, json};

/// The SDK's recorded and typed frames.
pub fn frames() -> BTreeMap<String, Value> {
    let text = include_str!("../../../ramflux-desktop-sdk/tests/fixtures/rfd_frames.json");
    serde_json::from_str(text).expect("fixture json")
}

/// Shell options on the SDK source at `path`, polling fast.
pub fn options(path: &std::path::Path) -> ShellOptions {
    let config = SdkConfig {
        deadlines: ramflux_desktop_sdk::Deadlines::uniform(Duration::from_secs(3)),
        poll_fast: Duration::from_millis(80),
        poll_slow: Duration::from_millis(300),
        reconnect_backoff: Duration::from_millis(150),
        ..SdkConfig::new(path)
    };
    ShellOptions {
        source: SourceChoice::Rfd(config),
        scenario: String::new(),
        start_clock: true,
        wall_clock: false,
        prefs: PrefsStore::in_memory(),
        launch: ramflux_desktop_ui::launch::LaunchOptions::in_memory(),
    }
}

/// Lets the bridge answer while the GPUI clock ticks, until `done` holds.
/// The wall-clock limit only bounds a failure.
pub fn wait(
    cx: &mut TestAppContext,
    what: &str,
    mut done: impl FnMut(&mut TestAppContext) -> bool,
) {
    let limit = std::time::Instant::now() + Duration::from_secs(30);
    while !done(cx) {
        assert!(std::time::Instant::now() < limit, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(10));
        crate::common::advance(cx, 50);
    }
}

/// What the fake account holds beyond the fixture.
#[derive(Default)]
pub struct State {
    /// Conversation rows added by submissions (and by the test).
    pub conversations: Vec<Value>,
    /// Message records by conversation.
    pub messages: BTreeMap<String, Vec<Value>>,
    /// Object rows added by submissions.
    pub objects: Vec<Value>,
    /// Make the next submissions fail with this message.
    pub fail_submit: Option<String>,
    /// Delay before a submission answers.
    pub submit_delay: Duration,
}

/// The shared state.
pub type Shared = Arc<Mutex<State>>;

fn contacts() -> Value {
    let link = |id: &str, target: &str| {
        json!({ "link_id": id, "requester_id": "principal-me", "target_id": target,
                "state": "accepted", "blocked": false })
    };
    json!({ "contacts": [
        link("l1", "principal-ana"), link("l2", "principal-bo"), link("l3", "principal-cy"),
        link("l4", "principal-dee"), link("l5", "principal-zed"),
    ] })
}

fn identities(all: &BTreeMap<String, Value>) -> Value {
    let mut frame = all["identity.directory"].clone();
    let rows = frame["identities"].as_array_mut().expect("rows");
    rows.push(json!({
        "id": "principal-dee", "principal_id": "principal-dee",
        "principal_commitment": "cm-dee", "contact_identity_commitment": "cm-dee",
        "display_name": "Dee", "kind": "human", "is_self": false,
        "devices": [{ "id": "device-dee", "device_id": "device-dee", "label": null,
                      "last_seen": null, "verified_at": null,
                      "target_delivery_id": "target-dee", "signing_public_key": null }],
        "safety_state": "unverified", "safety_number": null, "safety_number_available": false,
    }));
    frame
}

fn submitted(state: &mut State, body: &Value) -> Reply {
    let conversation = body["conversation_id"].as_str().unwrap_or_default().to_owned();
    let message = body["message_id"].as_str().unwrap_or_default().to_owned();
    if !state.conversations.iter().any(|c| c["id"] == conversation.as_str())
        && conversation != "dm-ana"
    {
        state.conversations.push(json!({
            "id": conversation, "archived": false, "hidden": false, "unread_count": 0,
            "is_unread": false, "muted": false, "disappearing": null, "kind": "direct",
            "title": body["title"], "participants": ["principal-me", "principal-bo"],
            "group_id": null, "self_principal_id": "principal-me",
            "direct_route": { "recipient_principal_commitment": body["recipient_principal_commitment"],
                              "recipient_device_ids": [body["recipient_device_id"]],
                              "target_delivery_id": body["target_delivery_id"], "federation": null },
        }));
    }
    let text = ramflux_protocol::decode_base64url(
        body["plaintext_body_base64"].as_str().unwrap_or_default(),
    )
    .unwrap_or_default();
    state.messages.entry(conversation.clone()).or_default().push(json!({
        "conversation_id": conversation, "message_id": message, "sender_id": "device-me",
        "encrypted_body": text, "deleted": false, "created_at": 1_790_000_500, "receipts": [],
    }));
    for attachment in body["attachments"].as_array().into_iter().flatten() {
        let size = ramflux_protocol::decode_base64url(
            attachment["plaintext_base64"].as_str().unwrap_or_default(),
        )
        .map_or(0, |b| b.len());
        state.objects.push(json!({
            "id": attachment["object_id"], "size": size, "transfer_state": "available",
            "name": attachment["name"], "direction": "outgoing", "conversation_id": conversation,
            "message_id": message, "transfer": null,
        }));
    }
    Reply::Ok(json!({ "message_id": message }))
}

/// Starts the fake daemon at a socket named after `name`.
pub fn start(name: &str) -> (FakeBus, Shared, std::path::PathBuf) {
    let path = socket_path(name);
    let shared: Shared = Arc::default();
    let state = shared.clone();
    let all = frames();
    let fake = FakeBus::start(&path, move |method, _, body| {
        if method == "message.submit" {
            let delay = state.lock().expect("state").submit_delay;
            std::thread::sleep(delay);
            let mut state = state.lock().expect("state");
            if let Some(message) = state.fail_submit.clone() {
                return Reply::Error("ValidationFailed", message);
            }
            return submitted(&mut state, body);
        }
        let state = state.lock().expect("state");
        let fixture = || all.get(method).cloned().unwrap_or(json!({}));
        Reply::Ok(match method {
            "contact.list" => contacts(),
            "identity.directory" => identities(&all),
            "conversation.directory" => {
                let mut frame = fixture();
                let rows = frame["conversations"].as_array_mut().expect("rows");
                rows.extend(state.conversations.iter().cloned());
                frame
            }
            "object.directory" => {
                let mut frame = fixture();
                frame["objects"]
                    .as_array_mut()
                    .expect("rows")
                    .extend(state.objects.iter().cloned());
                frame
            }
            "message.list" => {
                let conversation = body["conversation_id"].as_str().unwrap_or_default();
                let mut frame = fixture();
                if conversation != "dm-ana" {
                    frame["messages"] = json!([]);
                }
                let extra = state.messages.get(conversation).cloned().unwrap_or_default();
                frame["messages"].as_array_mut().expect("rows").extend(extra);
                frame
            }
            _ => fixture(),
        })
    });
    (fake, shared, path)
}
