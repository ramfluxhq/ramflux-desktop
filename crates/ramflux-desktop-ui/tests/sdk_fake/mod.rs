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
//! Agent surfaces are held on the protocol's state machine (`surfaces`),
//! voice calls by the call fake (`calls`), full backup and restore jobs by
//! the backup fake (`backup`).
//!
//! With `tasks` set, the daemon also holds conversation tasks (on the
//! protocol's state machine, see `fake_tasks`) and lists `approvals`; its
//! grant flow records each decision for the task's assignee to check.

#![allow(dead_code)]

pub mod author;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// While set, submissions wait to answer (the test releases them).
    pub submit_held: Arc<AtomicBool>,
    /// Agent surfaces the daemon holds.
    pub surfaces: Arc<ramflux_desktop_test::fake_surfaces::FakeSurfaces>,
    /// Conversation tasks the daemon holds.
    pub tasks: Option<ramflux_desktop_test::fake_tasks::FakeTasks>,
    /// Pending approvals listed instead of the fixture's, when any.
    pub approvals: Vec<Value>,
    /// Display names the identity directory reports, by principal.
    pub names: BTreeMap<String, String>,
    /// Voice calls (the CALL-01 API); `dm-ana` is a direct conversation
    /// with `principal-ana`.
    pub calls: Arc<ramflux_desktop_test::fake_calls::FakeCalls>,
    /// Contact requests (CONTACT-02, contract-first); a contact's row in
    /// `contact.list` carries this account's own request to it.
    pub requests: Arc<ramflux_desktop_test::fake_contact_requests::FakeContactRequests>,
    /// Full backup and restore jobs (the assumed BACKUP-01 API).
    pub backup: Arc<ramflux_desktop_test::fake_backup::FakeBackup>,
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

/// The approvals the test listed, and the grant flow's decisions on them.
fn approvals(
    state: &Shared,
    tasks: Option<&ramflux_desktop_test::fake_tasks::FakeTasks>,
    method: &str,
    body: &Value,
) -> Option<Reply> {
    let mut state = state.lock().expect("state");
    if state.approvals.is_empty() && tasks.is_none() {
        return None;
    }
    match method {
        "mcp.approval.list" if tasks.is_some() || !state.approvals.is_empty() => {
            Some(Reply::Ok(json!({ "approvals": state.approvals.clone(), "next_cursor": null })))
        }
        "grant.approve" | "grant.deny" => {
            let id = body["approval_id"].as_str().unwrap_or_default().to_owned();
            // A decision that was not delivered is sent again as it was.
            if let Some(sent) = tasks.and_then(|t| t.redeliver(&id, method == "grant.approve")) {
                return Some(if sent {
                    Reply::Ok(json!({ "approval_id": id, "decision_id": format!("mcpd_{id}") }))
                } else {
                    Reply::Error("ValidationFailed", "conflicting decision".to_owned())
                });
            }
            let position = state.approvals.iter().position(|r| r["approval_id"] == id.as_str())?;
            let record = state.approvals.remove(position);
            let decision = format!("mcpd_{id}");
            if let Some(tasks) = tasks {
                tasks.decide(&id, &decision, method == "grant.approve");
            }
            let grant = record["details"]["requested_grant_id"].clone();
            Some(Reply::Ok(
                json!({ "grant_id": grant, "approval_id": id, "decision_id": decision }),
            ))
        }
        _ => None,
    }
}

/// Starts the fake daemon at a socket named after `name`.
pub fn start(name: &str) -> (FakeBus, Shared, std::path::PathBuf) {
    let path = socket_path(name);
    let shared: Shared = Arc::default();
    let state = shared.clone();
    let all = frames();
    shared.lock().expect("state").calls.conversation("dm-ana", "principal-ana");
    let fake = FakeBus::start(&path, move |method, account, body| {
        let (surfaces, tasks, calls, requests, backup) = {
            let state = state.lock().expect("state");
            (
                state.surfaces.clone(),
                state.tasks.clone(),
                state.calls.clone(),
                state.requests.clone(),
                state.backup.clone(),
            )
        };
        if let Some(reply) = requests.handle(method, account, body) {
            return reply;
        }
        if let Some(reply) = backup.handle(method, account, body) {
            return reply;
        }
        if let Some(reply) = surfaces.handle(method, body) {
            return reply;
        }
        if let Some(reply) = calls.handle(method, account, body) {
            return reply;
        }
        if let Some(reply) = tasks.as_ref().and_then(|t| t.handle(method, body)) {
            return reply;
        }
        if let Some(reply) = approvals(&state, tasks.as_ref(), method, body) {
            return reply;
        }
        if method == "message.submit" {
            let (delay, held) = {
                let state = state.lock().expect("state");
                (state.submit_delay, state.submit_held.clone())
            };
            std::thread::sleep(delay);
            while held.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            let mut state = state.lock().expect("state");
            if let Some(message) = state.fail_submit.clone() {
                return Reply::Error("ValidationFailed", message);
            }
            return submitted(&mut state, body);
        }
        let state = state.lock().expect("state");
        let fixture = || all.get(method).cloned().unwrap_or(json!({}));
        Reply::Ok(match method {
            "contact.list" => {
                let mut listed = contacts();
                let requests = state.requests.clone();
                for row in listed["contacts"].as_array_mut().into_iter().flatten() {
                    let peer = row["target_id"].as_str().unwrap_or_default().to_owned();
                    if let Some(own) = requests.own_state("acct-main", &peer) {
                        row["principal_id"] = json!(peer);
                        row["request_state"] = json!(own);
                    }
                }
                listed
            }
            "identity.directory" => {
                let mut frame = identities(&all);
                for row in frame["identities"].as_array_mut().expect("rows") {
                    let principal = row["principal_id"].as_str().unwrap_or_default();
                    if let Some(name) = state.names.get(principal) {
                        row["display_name"] = json!(name);
                    }
                }
                frame
            }
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
                if let Some(tasks) = &tasks {
                    frame["messages"]
                        .as_array_mut()
                        .expect("rows")
                        .extend(tasks.messages(conversation));
                }
                frame
            }
            // A group's task messages come with its receive, as the
            // daemon lists them.
            "group.receive" => {
                let conversation = body["conversation_id"].as_str().unwrap_or_default();
                let mut frame = fixture();
                if let Some(tasks) = &tasks {
                    frame["messages"] = json!(tasks.messages(conversation));
                }
                frame
            }
            _ => fixture(),
        })
    });
    (fake, shared, path)
}

/// The bot member of the fixture's group, which plays the AI participant.
pub const BOT: &str = "principal-bo";
/// The fixture's group conversation.
pub const GROUP: &str = "thread-team";

/// Starts the fake daemon with conversation tasks: this account
/// (`principal-me`), the bot, and Ana as another member. Changes are
/// announced on the event stream, which is on.
pub fn start_with_tasks(
    name: &str,
) -> (FakeBus, Shared, std::path::PathBuf, ramflux_desktop_test::fake_tasks::FakeTasks) {
    use ramflux_desktop_test::fake_tasks::{FakeTasks, Member};
    let (fake, shared, path) = start(name);
    let tasks = FakeTasks::new(
        Member::human("principal-me", "device-me", 7),
        Member::agent(BOT, "device-bo", 9),
    );
    tasks.add_member(Member::human("principal-ana", "device-ana", 11));
    tasks.announce_to(fake.publisher(), "acct-main");
    shared.lock().expect("state").tasks = Some(tasks.clone());
    fake.enable_stream();
    (fake, shared, path, tasks)
}

/// A pending approval whose decision grants `grant_<id>`.
pub fn approval(id: &str, risk: &str) -> Value {
    json!({
        "approval_id": id, "status": "pending", "server_id": "deploy", "tool_name": "staging",
        "tool_scope": null, "risk_level": risk, "expires_at": 1_990_000_000,
        "details": { "arguments_hash": "sha256:5f1c09a2d7e4", "single_use": true,
                     "requested_grant_id": format!("grant_{id}") },
    })
}
