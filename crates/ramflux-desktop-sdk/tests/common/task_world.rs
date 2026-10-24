// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A fake daemon holding conversation tasks on the protocol's state
//! machine (see `ramflux_desktop_test::fake_tasks`), with the fixture's
//! account: this account (`principal-me`), the bot `principal-bo` in the
//! group `thread-team`, and Ana as another member. Its grant flow records
//! each approval's decision for the assignee to check.

use std::sync::{Arc, Mutex};

use ramflux_desktop_core::Source as _;
use ramflux_desktop_core::{
    AccountId, ContextId, Intent, IntentKind, Store, Task, TaskId, Timestamp, handle_intent,
};
use ramflux_desktop_test::fake_bus::Seen;
use ramflux_desktop_test::fake_surfaces::FakeSurfaces;
use ramflux_desktop_test::fake_tasks::{FakeTasks, Member};
use serde_json::{Value, json};

use super::{FakeBus, Fed, Reply, config, fixture, frames, socket_path};

pub const ACCOUNT: &str = "acct-main";
pub const GROUP: &str = "thread-team";
pub const BOT: &str = "principal-bo";

pub fn key(conversation: &str, task: &str) -> TaskId {
    ramflux_desktop_sdk::translate::tasks::task_key(conversation, task)
}

/// The daemon: the fixture, tasks, the assignee's surfaces, and the
/// approvals the test adds.
pub struct World {
    pub fake: FakeBus,
    pub tasks: FakeTasks,
    pub surfaces: Arc<FakeSurfaces>,
    pub approvals: Arc<Mutex<Vec<Value>>>,
}

pub fn approval(id: &str, risk: &str) -> Value {
    json!({
        "approval_id": id, "status": "pending", "server_id": "deploy", "tool_name": "staging",
        "tool_scope": null, "risk_level": risk, "expires_at": 1_990_000_000,
        "decision_id": decision_id(id),
        "details": { "arguments_hash": "sha256:aa", "single_use": true,
                     "requested_grant_id": format!("grant_{id}") },
    })
}

/// The original MCP decision id behind the approval `approval_id` (what
/// `grant.approve` answers and `task.resolve` names).
pub fn decision_id(approval_id: &str) -> String {
    format!("mcpd_{approval_id}")
}

impl World {
    pub fn start(name: &str) -> (Self, std::path::PathBuf) {
        let path = socket_path(&format!("tasks-{name}"));
        let tasks = FakeTasks::new(
            Member::human("principal-me", "device-me", 7),
            Member::agent(BOT, "device-bo", 9),
        );
        tasks.add_member(Member::human("principal-ana", "device-ana", 11));
        let approvals: Arc<Mutex<Vec<Value>>> = Arc::default();
        let surfaces = Arc::new(FakeSurfaces::default());
        let (t, a, s) = (tasks.clone(), approvals.clone(), surfaces.clone());
        let fake = FakeBus::start(&path, move |method, _, body| {
            if let Some(reply) = t.handle(method, body).or_else(|| s.handle(method, body)) {
                return reply;
            }
            match method {
                "mcp.approval.list" => {
                    let rows = a.lock().expect("approvals").clone();
                    Reply::Ok(json!({ "approvals": rows, "next_cursor": null }))
                }
                // The grant flow records the decision the assignee checks.
                "grant.approve" | "grant.deny" => {
                    let id = body["approval_id"].as_str().unwrap_or_default().to_owned();
                    t.decide(&id, &decision_id(&id), method == "grant.approve");
                    a.lock().expect("approvals").retain(|r| r["approval_id"] != id.as_str());
                    Reply::Ok(json!({ "grant_id": format!("grant_{id}"), "approval_id": id,
                                      "decision_id": decision_id(&id) }))
                }
                "message.list" | "group.receive" => {
                    let conversation = body["conversation_id"].as_str().unwrap_or_default();
                    let mut frame = frames()["message.list"].clone();
                    if conversation != "dm-ana" {
                        frame["messages"] = json!([]);
                    }
                    let rows = frame["messages"].as_array_mut().expect("rows");
                    rows.extend(t.messages(conversation));
                    Reply::Ok(frame)
                }
                _ => fixture(method),
            }
        });
        fake.enable_stream();
        tasks.announce_to(fake.publisher(), ACCOUNT);
        (Self { fake, tasks, surfaces, approvals }, path)
    }

    pub fn since(&self, mark: usize, method: &str) -> Vec<Seen> {
        self.fake.seen().into_iter().skip(mark).filter(|s| s.method == method).collect()
    }

    pub fn mark(&self) -> usize {
        self.fake.seen().len()
    }
}

/// The account source on the stream, looking at the group.
pub fn open(path: &std::path::Path) -> Fed {
    let hub = ramflux_desktop_sdk::SdkHub::start(config(path)).expect("bridge");
    let mut account = Fed::new(hub.account_source(AccountId::new(ACCOUNT)));
    account.until("the group", |s, _| s.context(&ContextId::new(GROUP)).is_some());
    account.source.focus(Some(&ContextId::new(GROUP)));
    account
}

pub fn task(store: &Store, conversation: &str, id: &str) -> Option<Task> {
    store.task(&key(conversation, id)).cloned()
}

/// Submits what the core policy decides for `intent`.
pub fn act(account: &mut Fed, intent: Intent) {
    let effects = handle_intent(&account.store, intent, Timestamp(0)).expect("allowed");
    for effect in effects {
        account.source.submit(effect);
    }
}

pub fn human(kind: IntentKind) -> Intent {
    Intent::human(kind)
}
