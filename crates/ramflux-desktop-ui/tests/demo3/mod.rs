// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Demo ③ step by step on a real node: Alice on two desktops (two devices
//! of one principal, each on its own daemon) and Orbit, an AI participant
//! (a headless AI account on a third daemon, played over its bus, see
//! `agent`):
//!
//! 1. three daemons on the node; Alice's two devices and Orbit's account;
//! 2. Alice and Orbit add each other; both desktops open the workspace;
//! 3. Alice writes to Orbit from the contacts dock;
//! 4. Alice asks Orbit for a task from the composer; Orbit accepts it;
//! 5. Orbit reaches a step that needs a single-use MCP approval: the step
//!    asks on both of Alice's desktops;
//! 6. Alice approves on her first desktop with the hold; Orbit's daemon
//!    commits the decision, the tool runs once, the task continues and
//!    finishes with its signed result summary, shown on both desktops;
//! 7. a second task: Alice denies on her second desktop; Orbit ends it
//!    with a signed summary saying it was not done.
//!
//! Each step asserts the desktops' stores (and the agent's daemon) and
//! takes light and dark screenshots.

pub mod agent;

use std::path::PathBuf;
use std::time::Duration;

use ramflux_desktop_core::{ContextId, GrantId, NeedsMe, StepState, Task, TaskId, TaskState};
use ramflux_desktop_sdk::translate::tasks::{task_key, task_moment_id};
use ramflux_desktop_test::live_daemon::LiveDaemon;
use ramflux_desktop_ui::route::Page;
use ramflux_desktop_ui::shell::ShellOptions;
use serde_json::{Value, json};

use crate::rehearsal::config::DemoConfig;
use crate::rehearsal::desk::Desk;
use crate::rehearsal::host::Host;
use crate::rehearsal::steps::{Notes, Steps};
use agent::{Account, Bus, Who};

/// Builds a desktop's options on a daemon socket.
pub type OptionsFor = Box<dyn Fn(&std::path::Path) -> ShellOptions>;

/// The rehearsal in progress.
pub struct Demo3<'h, H: Host> {
    host: &'h mut H,
    config: DemoConfig,
    options: OptionsFor,
    /// Alice's first and second device, and Orbit.
    daemons: [LiveDaemon; 3],
    accounts: [Option<Account>; 3],
    desks: [Option<Desk>; 2],
    conversation: Option<ContextId>,
    /// The approved task and the denied one (the daemon's task ids).
    approved: Option<String>,
}

const ORBIT: usize = 2;

impl<'h, H: Host> Demo3<'h, H> {
    pub fn new(
        host: &'h mut H,
        config: DemoConfig,
        options: OptionsFor,
        daemons: [LiveDaemon; 3],
    ) -> Self {
        Self {
            host,
            config,
            options,
            daemons,
            accounts: [None, None, None],
            desks: [None, None],
            conversation: None,
            approved: None,
        }
    }

    fn bus(&self, ix: usize) -> Bus<'_> {
        Bus { daemon: &self.daemons[ix], account: self.accounts[ix].clone() }
    }

    fn limit(&self) -> Duration {
        self.config.limits.answer
    }

    fn wait(
        &mut self,
        ix: usize,
        what: &str,
        done: impl FnMut(&Desk, &mut H) -> bool,
    ) -> Result<Duration, String> {
        let limit = self.limit();
        let desk = self.desks[ix].as_ref().ok_or("desktop not open")?;
        desk.until(self.host, limit, what, done)
    }

    fn shots(&mut self, ix: usize, name: &str, notes: &mut Notes) -> Result<(), String> {
        let Some(dir) = self.config.shots.clone() else { return Ok(()) };
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let desk = self.desks[ix].as_ref().ok_or("desktop not open")?;
        if desk.shots(self.host, &dir, name)? > 0 {
            notes.add(format!("{name}_{}_{{light,dark}}.png", desk.tag));
        }
        Ok(())
    }

    fn task(&mut self, ix: usize, id: &TaskId) -> Option<Task> {
        let desk = self.desks[ix].as_ref()?;
        desk.store(self.host, |s| s.task(id).cloned())
    }

    fn conversation(&self) -> Result<String, String> {
        self.conversation.as_ref().map(|c| c.to_string()).ok_or_else(|| "no conversation".into())
    }
}

/// Step 1: the node on all three daemons, Alice's two devices (one root)
/// and Orbit's headless AI account.
fn accounts<H: Host>(demo: &mut Demo3<'_, H>, notes: &mut Notes) -> Result<(), String> {
    let run = demo.config.run_id.clone();
    let root = agent::random();
    let alice = format!("principal-{}", agent::hex(&agent::random(), 16));
    let orbit = format!("principal-{}", agent::hex(&agent::random(), 16));
    let secret = crate::rehearsal::config::passphrase();
    let who = [
        ("acct-alice-1", alice.as_str(), format!("rehearsal-alice-{run}"), root, false),
        ("acct-alice-2", alice.as_str(), format!("rehearsal-alice-{run}"), root, false),
        ("acct-orbit", orbit.as_str(), format!("rehearsal-orbit-{run}"), agent::random(), true),
    ];
    for (ix, (account, principal, label, root, ai)) in who.into_iter().enumerate() {
        let mut bus = demo.bus(ix);
        let node = bus.node(&demo.config.node_url, &demo.config.node_key)?;
        let who = Who { account, principal, label: &label, root, agent: ai };
        let created = bus.create(&node, &who, &secret)?;
        notes.add(format!(
            "{account}: principal {}, device {} on {node}",
            &principal[..18.min(principal.len())],
            created["device_id"].as_str().unwrap_or("?")
        ));
        demo.accounts[ix] = bus.account;
    }
    Ok(())
}

/// Step 2: Alice and Orbit add each other; both of Alice's desktops open
/// on the workspace with Orbit as a contact.
fn contacts<H: Host>(demo: &mut Demo3<'_, H>, notes: &mut Notes) -> Result<(), String> {
    let alice = demo.accounts[0].clone().ok_or("no Alice")?.principal;
    let orbit = demo.accounts[ORBIT].clone().ok_or("no Orbit")?.principal;
    let added = demo.bus(0).add_contact(&orbit)?;
    notes.add(format!("Alice added Orbit: {}", added["state"]));
    let back = demo.bus(ORBIT).add_contact(&alice)?;
    notes.add(format!("Orbit added Alice: {}", back["state"]));
    for (ix, tag) in [(0, "a1"), (1, "a2")] {
        let options = (demo.options)(&demo.daemons[ix].socket());
        demo.desks[ix] = Some(Desk::open(demo.host, tag, options));
        demo.wait(ix, "the workspace", |desk, host| desk.page(host) == Page::Workspace)?;
    }
    let row = format!("contacts-{orbit}");
    demo.wait(0, "Orbit in Alice's contacts", |desk, host| {
        desk.store(host, |s| {
            s.participant(&ramflux_desktop_core::ParticipantId::new(&orbit)).is_some()
        })
    })?;
    let kind = demo.desks[0].as_ref().ok_or("desktop not open")?.store(demo.host, |s| {
        s.participant(&ramflux_desktop_core::ParticipantId::new(&orbit)).map(|p| p.kind)
    });
    notes.add(format!("Alice's desktop knows Orbit as {kind:?}"));
    let desk = demo.desks[0].as_ref().ok_or("desktop not open")?;
    if !desk.exists(demo.host, &row) {
        desk.click(demo.host, None, "rail-contacts")?;
    }
    demo.shots(0, "d3_02_contacts", notes)?;
    Ok(())
}

/// Step 3: Alice writes to Orbit from the contacts dock (the desktop
/// starts the conversation); Orbit's daemon has it.
fn conversation<H: Host>(demo: &mut Demo3<'_, H>, notes: &mut Notes) -> Result<(), String> {
    let orbit = demo.accounts[ORBIT].clone().ok_or("no Orbit")?.principal;
    let run = demo.config.run_id.clone();
    let row = format!("contacts-{orbit}");
    let desk = demo.desks[0].as_ref().ok_or("desktop not open")?;
    if !desk.exists(demo.host, &row) {
        desk.click(demo.host, None, "rail-contacts")?;
    }
    desk.click(demo.host, Some(&row), "message")?;
    if !desk.draft_open(demo.host) {
        return Err("Message on Orbit opened no draft".into());
    }
    let text = format!("Hi Orbit, build 2.4.0-rc1 is ready ({run})");
    desk.type_into(demo.host, "composer-input", &text)?;
    desk.click(demo.host, None, "composer-send")?;
    demo.wait(0, "the started conversation", |desk, host| {
        !desk.draft_open(host) && desk.current_context(host).is_some()
    })?;
    let context = demo.desks[0]
        .as_ref()
        .ok_or("desktop not open")?
        .current_context(demo.host)
        .ok_or("no conversation")?;
    notes.add(format!("conversation {context}"));
    demo.conversation = Some(context.clone());
    demo.bus(ORBIT).conversation(context.as_str(), demo.limit())?;
    notes.add("Orbit's daemon has the conversation");
    let seen = demo.wait(1, "the conversation on Alice's second device", |desk, host| {
        desk.store(host, |s| s.context(&context).is_some())
    });
    match seen {
        Ok(_) => {
            demo.desks[1].as_ref().ok_or("desktop not open")?.show_context(demo.host, &context)?;
            notes.add("Alice's second desktop shows the conversation too");
        }
        Err(e) => notes.add(format!("second desktop: {e}")),
    }
    demo.shots(0, "d3_03_conversation", notes)?;
    Ok(())
}

/// Asks Orbit for a task from Alice's first desktop: the composer's task
/// mode (`task.start`); when the desktop offers none (Orbit's kind not
/// known), as a message Orbit answers with a proposal Alice starts.
/// Returns the daemon's task (on Orbit's side) once running.
fn ask<H: Host>(demo: &mut Demo3<'_, H>, title: &str, notes: &mut Notes) -> Result<Value, String> {
    let conversation = demo.conversation()?;
    let alice = demo.accounts[0].clone().ok_or("no Alice")?.principal;
    let before: Vec<String> = known_tasks(&demo.bus(ORBIT), &conversation)?;
    let desk = demo.desks[0].as_ref().ok_or("desktop not open")?;
    let context = ContextId::new(&conversation);
    desk.show_context(demo.host, &context)?;
    let task_mode = desk.exists(demo.host, "composer-mode");
    if task_mode {
        desk.click(demo.host, None, "composer-mode")?;
    }
    desk.type_into(demo.host, "composer-input", title)?;
    desk.click(demo.host, None, "composer-send")?;
    let fresh = |t: &Value| !before.iter().any(|b| t["task_id"] == b.as_str());
    let limit = demo.limit();
    let started = demo.bus(ORBIT).task_at(&conversation, 1, Duration::from_secs(30), |t| {
        fresh(t) && t["requester"] == alice.as_str()
    });
    if let Ok(task) = started {
        notes.add(format!("asked from the composer's task mode: task.start {}", task["task_id"]));
        return Ok(task);
    }
    // Orbit proposes what Alice asked for; Alice starts it on her desktop.
    notes.add(format!(
        "the composer offered {} task mode; Orbit proposes, Alice starts it",
        if task_mode { "a" } else { "no" }
    ));
    let id = format!("t-{}", agent::hex(&agent::random(), 6));
    let orbit = demo.accounts[ORBIT].clone().ok_or("no Orbit")?.principal;
    demo.bus(ORBIT).call(
        "task",
        "task.propose",
        &json!({ "conversation_id": conversation, "task_id": id, "assignee": orbit,
                 "title": title, "steps": [{ "id": "build", "title": "Check the build",
                 "state": "proposed" }, { "id": "deploy", "title": "Deploy to staging",
                 "state": "proposed" }] }),
    )?;
    let key = task_key(&conversation, &id);
    demo.wait(0, "the proposal", |desk, host| desk.store(host, |s| s.task(&key).is_some()))?;
    let scope = format!("moment-{}", task_moment_id(&key));
    demo.shots(0, "d3_04_proposal", notes)?;
    demo.desks[0].as_ref().ok_or("desktop not open")?.click(
        demo.host,
        Some(&scope),
        "task-start",
    )?;
    demo.bus(ORBIT).task_at(&conversation, 2, limit, |t| t["task_id"] == id.as_str())
}

fn known_tasks(bus: &Bus<'_>, conversation: &str) -> Result<Vec<String>, String> {
    let listed = bus.call(
        "task",
        "task.list",
        &json!({ "conversation_id": conversation, "cursor": "", "limit": 50 }),
    )?;
    Ok(listed["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["task_id"].as_str().map(str::to_owned))
        .collect())
}

/// Orbit accepts the task and runs it up to the step that needs Alice's
/// approval; returns (task view, approval, grant id).
fn to_approval<H: Host>(
    demo: &mut Demo3<'_, H>,
    task: Value,
    args: &Value,
    notes: &mut Notes,
) -> Result<(Value, Value, String), String> {
    let conversation = demo.conversation()?;
    let orbit = demo.bus(ORBIT);
    let steps = json!([
        { "id": "build", "title": "Check build 2.4.0-rc1", "state": "done" },
        { "id": "deploy", "title": "Deploy to staging", "state": "running" },
    ]);
    let version = orbit.update(&task, "running", &steps, None)?;
    let task =
        orbit.task_at(&conversation, version, demo.limit(), |t| t["task_id"] == task["task_id"])?;
    let approval = orbit.ask_approval(args)?;
    let grant = agent::requested_grant(&approval);
    let steps = json!([
        { "id": "build", "title": "Check build 2.4.0-rc1", "state": "done" },
        { "id": "deploy", "title": "Deploy to staging", "state": "needs_me",
          "needs_me": { "kind": "approval", "grant_id": grant } },
    ]);
    let version = orbit.update(&task, "needs_me", &steps, None)?;
    let task =
        orbit.task_at(&conversation, version, demo.limit(), |t| t["task_id"] == task["task_id"])?;
    notes.add(format!(
        "Orbit accepted and ran {} to v{version}; the tool asks for approval {} (grant {grant})",
        task["task_id"], approval["approval_id"]
    ));
    Ok((task, approval, grant))
}

/// Waits until desktop `ix` shows the step asking for the approval, with
/// the grant it waits on known (decidable in place).
fn asking<H: Host>(demo: &mut Demo3<'_, H>, ix: usize, key: &TaskId) -> Result<GrantId, String> {
    let key2 = key.clone();
    demo.wait(ix, "the step asking for approval", move |desk, host| {
        desk.store(host, |s| {
            s.task(&key2).is_some_and(|t| {
                t.state == TaskState::NeedsMe
                    && t.steps.iter().any(|st| {
                        st.state == StepState::NeedsMe
                            && matches!(&st.needs_me, Some(NeedsMe::Approval { grant_id })
                                if s.grant(grant_id).is_some())
                    })
            })
        })
    })
    .map_err(|e| {
        let seen = demo.desks[ix].as_ref().map(|desk| {
            desk.store(demo.host, |s| {
                let task = s.task(key).map(|t| {
                    let steps: Vec<String> = t
                        .steps
                        .iter()
                        .map(|st| format!("{}={:?}/{:?}", st.id, st.state, st.needs_me))
                        .collect();
                    format!("v{} {:?} [{}]", t.version, t.state, steps.join(", "))
                });
                let grants: Vec<String> =
                    s.grants().map(|g| format!("{}:{:?}", g.id, g.state)).collect();
                format!(
                    "task {task:?}; grants {grants:?}; failures {:?}",
                    s.failures()
                        .iter()
                        .map(|f| format!("{}: {}", f.method, f.message))
                        .collect::<Vec<_>>()
                )
            })
        });
        format!("{e}; desktop saw {}", seen.unwrap_or_default())
    })?;
    let task = demo.task(ix, key).ok_or("task gone")?;
    task.steps
        .iter()
        .find_map(|s| match &s.needs_me {
            Some(NeedsMe::Approval { grant_id }) if s.state == StepState::NeedsMe => {
                Some(grant_id.clone())
            }
            _ => None,
        })
        .ok_or_else(|| "no approval step".into())
}

/// Steps 4 to 6: ask, accept, approve with the hold, continue, done.
fn approve<H: Host>(demo: &mut Demo3<'_, H>, notes: &mut Notes) -> Result<(), String> {
    let run = demo.config.run_id.clone();
    let task = ask(demo, &format!("Deploy build 2.4.0-rc1 to staging ({run})"), notes)?;
    let conversation = demo.conversation()?;
    let id = task["task_id"].as_str().ok_or("task id")?.to_owned();
    let key = task_key(&conversation, &id);
    demo.wait(0, "the task card", |desk, host| desk.store(host, |s| s.task(&key).is_some()))?;
    demo.shots(0, "d3_04_task_running", notes)?;
    let args = json!({ "build": "2.4.0-rc1", "target": "staging", "run": run });
    let (task, _approval, _grant) = to_approval(demo, task, &args, notes)?;
    let grant = asking(demo, 0, &key)?;
    notes.add(format!("first desktop: the step asks for {grant}, decidable in place"));
    demo.shots(0, "d3_05_needs_me_approval", notes)?;
    match asking(demo, 1, &key) {
        Ok(_) => {
            notes.add("second desktop: the same step asks there too");
            demo.shots(1, "d3_05_needs_me_approval", notes)?;
        }
        Err(e) => notes.add(format!("second desktop: {e}")),
    }
    let scope = format!("moment-{}", task_moment_id(&key));
    let desk = demo.desks[0].as_ref().ok_or("desktop not open")?;
    desk.click(demo.host, Some(&scope), "step-approve")?;
    if !desk.strip_open(demo.host) {
        return Err("a high-risk approval opened no hold".into());
    }
    demo.shots(0, "d3_06_hold", notes)?;
    demo.desks[0].as_ref().ok_or("desktop not open")?.hold_confirm(demo.host)?;
    let version = task["version"].as_u64().unwrap_or(0);
    demo.wait(0, "the decision on its way", |desk, host| {
        desk.store(host, |s| {
            s.task(&key).is_some_and(|t| t.answer.is_some() || t.version > version)
        })
    })?;
    demo.shots(0, "d3_06_decision_sent", notes)?;
    // Orbit's daemon checks the signed grant and commits the decision.
    let orbit = demo.bus(ORBIT);
    let committed =
        orbit.task_at(&conversation, version + 1, demo.limit(), |t| t["task_id"] == id.as_str())?;
    notes.add(format!("Orbit's daemon committed the approval at v{}", committed["version"]));
    let ran = orbit.tool(&args)?;
    notes.add(format!("the tool ran once approved: {}", ran["status"]));
    if ran["status"] != "ok" {
        return Err(format!("the approved tool did not run: {ran}"));
    }
    let again = orbit.tool(&args)?;
    notes.add(format!("a second run needs a new approval: {}", again["status"]));
    let summary = format!("Deployed 2.4.0-rc1 to staging; health checks pass ({run})");
    let steps = json!([
        { "id": "build", "title": "Check build 2.4.0-rc1", "state": "done" },
        { "id": "deploy", "title": "Deploy to staging", "state": "done" },
    ]);
    let done = orbit.update(&committed, "done", &steps, Some(&summary))?;
    notes.add(format!("Orbit finished the task at v{done} with a signed summary"));
    for ix in 0..2 {
        let s = summary.clone();
        let shown = demo.wait(ix, "done with its summary", |desk, host| {
            desk.store(host, |st| {
                st.task(&key)
                    .is_some_and(|t| t.state == TaskState::Done && t.result.as_deref() == Some(&s))
            })
        });
        match (ix, shown) {
            (_, Ok(_)) => demo.shots(ix, "d3_07_done", notes)?,
            (0, Err(e)) => return Err(e),
            (_, Err(e)) => notes.add(format!("second desktop: {e}")),
        }
    }
    demo.approved = Some(id);
    Ok(())
}

/// Step 7: a second task; Alice denies on her second desktop (on the first
/// when the second does not show it); Orbit ends it without running.
fn deny<H: Host>(demo: &mut Demo3<'_, H>, notes: &mut Notes) -> Result<(), String> {
    let run = demo.config.run_id.clone();
    let task = ask(demo, &format!("Deploy build 2.4.0-rc1 to production ({run})"), notes)?;
    let conversation = demo.conversation()?;
    let id = task["task_id"].as_str().ok_or("task id")?.to_owned();
    let key = task_key(&conversation, &id);
    let args = json!({ "build": "2.4.0-rc1", "target": "production", "run": run });
    let (task, _approval, _grant) = to_approval(demo, task, &args, notes)?;
    asking(demo, 0, &key)?;
    let on = match asking(demo, 1, &key) {
        Ok(_) => 1,
        Err(e) => {
            notes.add(format!("second desktop: {e}; denied on the first"));
            0
        }
    };
    demo.shots(on, "d3_08_needs_me_deny", notes)?;
    let scope = format!("moment-{}", task_moment_id(&key));
    demo.desks[on].as_ref().ok_or("desktop not open")?.click(
        demo.host,
        Some(&scope),
        "step-deny",
    )?;
    notes.add(format!(
        "denied on desktop {}",
        demo.desks[on].as_ref().ok_or("desktop not open")?.tag
    ));
    let version = task["version"].as_u64().unwrap_or(0);
    let orbit = demo.bus(ORBIT);
    let committed =
        orbit.task_at(&conversation, version + 1, demo.limit(), |t| t["task_id"] == id.as_str())?;
    notes.add(format!("Orbit's daemon committed the denial at v{}", committed["version"]));
    let ran = orbit.tool(&args)?;
    notes.add(format!("the tool after the denial: {}", ran["status"]));
    if ran["status"] == "ok" {
        return Err("the denied tool ran".into());
    }
    let summary = format!("Not deployed to production: Alice denied the approval ({run})");
    let steps = json!([
        { "id": "build", "title": "Check build 2.4.0-rc1", "state": "done" },
        { "id": "deploy", "title": "Deploy to production", "state": "cancelled" },
    ]);
    let state = match committed["state"].as_str() {
        Some("failed" | "cancelled" | "done") => None,
        _ => Some("failed"),
    };
    if let Some(state) = state {
        orbit.update(&committed, state, &steps, Some(&summary))?;
        notes.add(format!("Orbit ended the task as {state} with a signed summary"));
    } else {
        notes.add(format!("the denial ended the task as {}", committed["state"]));
    }
    for ix in 0..2 {
        let shown = demo.wait(ix, "the denied task ended", |desk, host| {
            desk.store(host, |st| st.task(&key).is_some_and(|t| t.state.is_terminal()))
        });
        match (ix, shown) {
            (_, Ok(_)) => {
                let task = demo.task(ix, &key).ok_or("task gone")?;
                notes.add(format!("desktop {ix}: {:?}, result {:?}", task.state, task.result));
                demo.shots(ix, "d3_09_denied", notes)?;
            }
            (0, Err(e)) => return Err(e),
            (_, Err(e)) => notes.add(format!("second desktop: {e}")),
        }
    }
    Ok(())
}

/// Runs every step and returns the table.
pub fn run<H: Host>(demo: &mut Demo3<'_, H>) -> Steps {
    let mut steps = Steps::default();
    steps.run(1, "Node, Alice's two devices and Orbit (headless AI) on three daemons", &[], |n| {
        accounts(demo, n)
    });
    steps.run(2, "Alice and Orbit add each other; both desktops open", &[1], |n| contacts(demo, n));
    steps.run(3, "Alice writes to Orbit from the contacts dock", &[2], |n| conversation(demo, n));
    steps.run(
        4,
        "Ask a task; Orbit accepts; approval asked on both desktops; approve with the hold; the task continues and finishes with its signed summary",
        &[3],
        |n| approve(demo, n),
    );
    steps.run(
        5,
        "A second task: deny on the second desktop; Orbit ends it, nothing ran",
        &[3],
        |n| deny(demo, n),
    );
    steps
}

/// The daemons' directories, for the report.
pub fn dirs(demo: &Demo3<'_, impl Host>) -> Vec<PathBuf> {
    demo.daemons.iter().map(|d| d.dir().to_path_buf()).collect()
}
