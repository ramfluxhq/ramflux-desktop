// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Conversation tasks as the daemon holds them, for adapter and UI tests.
//!
//! Every change is a task event signed by its member's key and applied by
//! the protocol's own state machine (`TaskState::apply`), so authority,
//! revisions, single-use decisions and transitions are the protocol's, not
//! a copy of them. On top, like the daemon:
//!
//! - tasks are scoped by (conversation, task id);
//! - requests use the daemon's bodies, unknown fields are refused, and a
//!   refusal is an error frame with the daemon's typed `error.code`
//!   (`stale_revision`, `terminal`, `not_requester`, `decision_consumed`,
//!   ...; the protocol's refusals map as the daemon maps them) and a text
//!   nobody parses;
//! - acceptance has one form, `task.accept {task_id, version}`;
//!   `proposal_id` is refused;
//! - `task.get` / `task.list` answer the daemon's view (the protocol task,
//!   with its signed `result_summary`, plus `id`, `actor_kind_verified:
//!   false`, per-actor `actors.{requester,assignee,acceptor}`, times, the
//!   stalled and compacted markers, and `pending_resolution`);
//! - every accepted change is announced as `task.changed` (metadata only,
//!   typed by `kind`: `updated`, `compacted` or `removed`) and leaves a
//!   `content_type: "task"` message with its `task_id` and fallback text;
//!   compaction and pruning are announced too;
//! - the requester's decision on a step's approval has a delivery state
//!   (`task.get` `approval_decisions[]`): [`FakeTasks::fail_delivery`]
//!   marks it `failed` (retries ran out), and the same `grant.approve` /
//!   `grant.deny` again resends it ([`FakeTasks::redeliver`]);
//! - a requester's resolution is pending until the assignee confirms it,
//!   which it does only for a decision the grant flow recorded
//!   ([`FakeTasks::decide`], [`FakeTasks::confirm_decisions`]); until then
//!   the view carries `pending_resolution` (`awaiting_executor`, or
//!   `executor_timeout` after [`FakeTasks::time_out_decisions`]) naming the
//!   step it decides (`step_id`).
//!
//! The assignee (an AI participant) is played by the test through
//! [`FakeTasks::propose`] and [`FakeTasks::update`].

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use ramflux_protocol::{
    ActorType, NeedsMe, SignatureAlg, SignedFields, TaskAccept, TaskCancel, TaskDecline, TaskEvent,
    TaskEventBody, TaskProposal, TaskReject, TaskResolve, TaskSigner, TaskStart, TaskState,
    TaskStatus, TaskStep, TaskUpdate,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::fake_bus::{Publisher, Reply};

/// A member of a conversation with its own signing key.
#[derive(Clone, Debug)]
pub struct Member {
    /// Principal id.
    pub principal: String,
    /// Device id (also the signing key id).
    pub device: String,
    seed: [u8; 32],
    kind: ActorType,
}

impl Member {
    /// A person.
    pub fn human(principal: &str, device: &str, seed: u8) -> Self {
        Self::new(principal, device, seed, ActorType::Human)
    }

    /// An AI participant (a bot account).
    pub fn agent(principal: &str, device: &str, seed: u8) -> Self {
        Self::new(principal, device, seed, ActorType::Bot)
    }

    fn new(principal: &str, device: &str, seed: u8, kind: ActorType) -> Self {
        Self { principal: principal.to_owned(), device: device.to_owned(), seed: [seed; 32], kind }
    }

    fn public(&self) -> String {
        ramflux_crypto::public_key_base64url_from_seed(self.seed)
    }

    fn sign(&self, conversation: &str, body: TaskEventBody) -> Result<TaskEvent, Refused> {
        self.signed(conversation, body).map_err(|e| ("invalid_event", e))
    }

    fn signed(&self, conversation: &str, body: TaskEventBody) -> Result<TaskEvent, String> {
        let signed = SignedFields {
            signing_key_id: self.device.clone(),
            signature_alg: SignatureAlg::Ed25519,
            signature: String::new(),
        };
        let event = TaskEvent::new(conversation.to_owned(), body, signed).map_err(text)?;
        let signature =
            ramflux_crypto::sign_protocol_object_with_seed(&event, self.seed).map_err(text)?;
        let event = event.with_signature(signature).map_err(text)?;
        // Received as the daemon receives it: from the original bytes.
        let bytes = serde_json::to_vec(&event).map_err(text)?;
        TaskEvent::from_wire_bytes(&bytes).map_err(text)
    }
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// A refusal: the daemon's typed code and its text.
type Refused = (&'static str, String);

fn invalid(message: impl std::fmt::Display) -> Refused {
    ("invalid_request", message.to_string())
}

/// The daemon's code for a protocol refusal.
fn reject_code(reject: &TaskReject) -> &'static str {
    match reject {
        TaskReject::StaleVersion { .. }
        | TaskReject::StaleDecision { .. }
        | TaskReject::InvalidVersion => "stale_revision",
        TaskReject::Terminal => "terminal",
        TaskReject::HumanRequired => "human_required",
        TaskReject::NotFound => "not_found",
        TaskReject::AlreadyExists => "already_exists",
        TaskReject::DecisionReplay => "decision_consumed",
        TaskReject::DecisionMismatch | TaskReject::HumanDecisionPending => "decision_mismatch",
        _ => "invalid_event",
    }
}

fn rejected(reject: &TaskReject) -> Refused {
    (reject_code(reject), reject.to_string())
}

/// One task as the daemon holds it.
#[derive(Default)]
struct Held {
    state: TaskState,
    created_at: i64,
    updated_at: i64,
    stalled: bool,
    compacted: bool,
    /// Requester resolutions waiting for the assignee, by decision id.
    pending: BTreeMap<String, (TaskEvent, Member)>,
    /// The pending resolution's message id, queue time and step.
    pending_since: Option<(String, i64, String)>,
    /// The pending resolution outlived the requester's wait.
    timed_out: bool,
}

#[derive(Default)]
struct Inner {
    local: Option<Member>,
    members: Vec<Member>,
    tasks: BTreeMap<(String, String), Held>,
    messages: BTreeMap<String, Vec<Value>>,
    /// Grant-flow decisions the assignee can verify, by approval id.
    decided: BTreeMap<String, bool>,
    /// Scoped approval ids and the decision ids they stand for.
    aliases: BTreeMap<String, String>,
    announce: Option<(Publisher, String)>,
    quiet: bool,
    /// Also announce each task message as `message.received`.
    message_events: bool,
    /// Task messages are not listed or announced until released.
    messages_held: bool,
    /// Whether task message rows carry the signer's authenticated kind
    /// (`actor_kind`) and the role-aware `actor_kind_verified`, as the
    /// daemon does (KIND-01-FU-02); off, rows carry `false` alone.
    event_kinds: bool,
    clock: i64,
    ids: u64,
    refuse_next: Option<(String, Refused)>,
    /// Principals whose kind a root-signed profile proves.
    verified: Vec<String>,
    /// The kind each principal's profile states (`human`, `headless_ai`,
    /// `bot`), when the daemon holds one.
    kinds: BTreeMap<String, &'static str>,
    /// The requester's approval decisions and their delivery, by approval
    /// id: (conversation, task, step), approved, delivery state.
    deliveries: BTreeMap<String, Delivery>,
}

/// One approval decision on its way to the assignee.
#[derive(Clone)]
struct Delivery {
    conversation: String,
    task: String,
    step: String,
    approved: bool,
    state: &'static str,
}

/// The daemon's tasks of one account. Cheap to clone; clones share state.
#[derive(Clone, Default)]
pub struct FakeTasks(Arc<Mutex<Inner>>);

/// The daemon's error frame for a refused task request.
fn refused((code, message): Refused) -> Reply {
    Reply::Error(code, format!("local bus error: {message}"))
}

/// Parses a request body into a protocol payload after removing the
/// fields the daemon handles itself; anything else unknown is refused.
fn payload<T: DeserializeOwned>(body: &Value, drop: &[&str]) -> Result<T, Refused> {
    let mut body = body.clone();
    if let Some(map) = body.as_object_mut() {
        for field in drop {
            map.remove(*field);
        }
    }
    serde_json::from_value(body).map_err(invalid)
}

fn text_of<'a>(body: &'a Value, name: &str) -> &'a str {
    body[name].as_str().unwrap_or_default()
}

fn field<'a>(body: &'a Value, name: &str) -> Result<&'a str, Refused> {
    body[name]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| invalid(format!("task {name} required")))
}

impl FakeTasks {
    /// Tasks of the account of `local` (the attended person), with `agent`
    /// as a member who works on them.
    pub fn new(local: Member, agent: Member) -> Self {
        let tasks = Self::default();
        {
            let mut inner = tasks.inner();
            inner.members = vec![local.clone(), agent];
            inner.local = Some(local);
            inner.clock = 1_790_001_000;
        }
        tasks
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().expect("tasks")
    }

    /// Adds another member (a person who can start or accept tasks).
    pub fn add_member(&self, member: Member) {
        self.inner().members.push(member);
    }

    /// Announces every change as `task.changed` on `account`'s journal.
    pub fn announce_to(&self, publisher: Publisher, account: &str) {
        self.inner().announce = Some((publisher, account.to_owned()));
    }

    /// Also announces each task message as `message.received` (metadata:
    /// the message and its conversation), as the daemon announces every
    /// message it projects; off by default, so the tests written before it
    /// keep the reads they count.
    pub fn announce_messages(&self, on: bool) {
        self.inner().message_events = on;
    }

    /// Makes task message rows carry the signer's authenticated kind
    /// (`actor_kind`, from [`FakeTasks::set_kind`]) and the role-aware
    /// `actor_kind_verified` (from [`FakeTasks::verify_kind`]).
    pub fn report_event_kinds(&self, report: bool) {
        self.inner().event_kinds = report;
    }

    /// Holds back the task messages (they are neither listed nor
    /// announced) or releases them: the daemon projects a message some time
    /// after the task changed, so a desktop may read the changed task first.
    /// Releasing announces each held message when announcing is on.
    pub fn hold_messages(&self, hold: bool) {
        let mut inner = self.inner();
        inner.messages_held = hold;
        if hold || !inner.message_events {
            return;
        }
        let Some((publisher, account)) = inner.announce.clone() else { return };
        for (conversation, rows) in &inner.messages {
            for row in rows.iter().filter(|r| r["content_type"] == "task") {
                publisher.publish(
                    &account,
                    "message.received",
                    json!({ "message": { "id": row["message_id"], "conversation_id": conversation },
                            "conversation": { "id": conversation } }),
                );
            }
        }
    }

    /// Stops (or resumes) announcing changes: a change the desktop has not
    /// heard of yet, as when another device acted a moment earlier.
    pub fn set_quiet(&self, quiet: bool) {
        self.inner().quiet = quiet;
    }

    /// Refuses the next request of `method` with the daemon's typed
    /// `code` and a `message` (which the desktop must not need).
    pub fn refuse_next(&self, method: &str, code: &'static str, message: &str) {
        self.inner().refuse_next = Some((method.to_owned(), (code, message.to_owned())));
    }

    /// Records the grant flow's decision on the approval `approval_id`
    /// (the scoped id the desktop shows) whose original MCP decision id is
    /// `decision_id`; the assignee checks it before it confirms a
    /// resolution naming it. Like the daemon, a resolution naming the
    /// scoped approval id is canonicalized to the decision id.
    pub fn decide(&self, approval_id: &str, decision_id: &str, approved: bool) {
        let mut inner = self.inner();
        inner.decided.insert(decision_id.to_owned(), approved);
        inner.aliases.insert(approval_id.to_owned(), decision_id.to_owned());
    }

    /// The assignee confirms every pending resolution whose decision the
    /// grant flow recorded as the resolution says; one that disagrees with
    /// the recorded decision is rejected (`decision_rejected`). Returns how
    /// many it confirmed.
    pub fn confirm_decisions(&self) -> usize {
        let mut inner = self.inner();
        let decided = inner.decided.clone();
        let mut confirmed = Vec::new();
        let mut rejected = Vec::new();
        for (key, held) in &mut inner.tasks {
            let ready: Vec<String> =
                held.pending.keys().filter(|id| decided.contains_key(*id)).cloned().collect();
            for id in ready {
                let Some((event, member)) = held.pending.remove(&id) else { continue };
                let resume = matches!(event.event(), TaskEventBody::Resolve(r)
                    if r.decision == ramflux_protocol::TaskDecision::Resume);
                if decided.get(&id) == Some(&resume)
                    && apply(&mut held.state, &event, &member).is_ok()
                {
                    confirmed.push((key.clone(), event, member));
                } else {
                    rejected.push(key.clone());
                }
            }
            if held.pending.is_empty() {
                held.pending_since = None;
                held.timed_out = false;
            }
        }
        let count = confirmed.len();
        for ((conversation, task), event, member) in confirmed {
            inner.changed(&conversation, &task, &event, &member);
        }
        for (conversation, task) in rejected {
            inner.announce(json!({ "conversation_id": conversation, "task_id": task,
                                   "decision_rejected": true, "actor_kind_verified": false }));
        }
        count
    }

    /// The AI participant proposes a task it would do, with these steps
    /// (id, title).
    ///
    /// # Errors
    ///
    /// The protocol's refusal.
    pub fn propose(
        &self,
        conversation: &str,
        task_id: &str,
        title: &str,
        steps: &[(&str, &str)],
    ) -> Result<u64, String> {
        let mut inner = self.inner();
        let member = inner.agent().map_err(|(_, m)| m)?;
        let steps =
            steps.iter().map(|(id, title)| step(id, title, TaskStatus::Proposed, None)).collect();
        let body = TaskEventBody::Proposal(TaskProposal {
            task_id: task_id.to_owned(),
            conversation_id: conversation.to_owned(),
            proposer: member.principal.clone(),
            assignee: member.principal.clone(),
            title: title.to_owned(),
            steps,
        });
        inner.commit(conversation, task_id, &member, body).map_err(|(_, m)| m)
    }

    /// The task's assignee updates it to `state` with `steps` (the next
    /// revision). A stalled assignee is refused, as the daemon refuses it.
    ///
    /// # Errors
    ///
    /// The protocol's (or the quota's) refusal.
    pub fn update(
        &self,
        conversation: &str,
        task_id: &str,
        state: TaskStatus,
        steps: Vec<TaskStep>,
        result_ref: Option<&str>,
    ) -> Result<u64, String> {
        self.assignee_update((conversation, task_id), state, steps, (result_ref, None))
    }

    /// The assignee finishes the task in the terminal `state` with its
    /// signed result summary (the protocol allows one on a terminal
    /// update only).
    ///
    /// # Errors
    ///
    /// The protocol's (or the quota's) refusal.
    pub fn finish(
        &self,
        conversation: &str,
        task_id: &str,
        state: TaskStatus,
        steps: Vec<TaskStep>,
        summary: &str,
    ) -> Result<u64, String> {
        self.assignee_update((conversation, task_id), state, steps, (None, Some(summary)))
    }

    /// Like [`Self::finish`], and the result also points at the surface
    /// `result_ref` that presents it.
    ///
    /// # Errors
    ///
    /// The protocol's (or the quota's) refusal.
    pub fn finish_with_surface(
        &self,
        conversation: &str,
        task_id: &str,
        state: TaskStatus,
        steps: Vec<TaskStep>,
        (result_ref, summary): (&str, &str),
    ) -> Result<u64, String> {
        self.assignee_update(
            (conversation, task_id),
            state,
            steps,
            (Some(result_ref), Some(summary)),
        )
    }

    fn assignee_update(
        &self,
        (conversation, task_id): (&str, &str),
        state: TaskStatus,
        steps: Vec<TaskStep>,
        (result_ref, result_summary): (Option<&str>, Option<&str>),
    ) -> Result<u64, String> {
        let mut inner = self.inner();
        let key = (conversation.to_owned(), task_id.to_owned());
        let held = inner.tasks.get(&key).ok_or("task does not exist")?;
        let held_stalled = held.stalled;
        let assignee = held.state.task().map(|t| t.assignee.clone()).unwrap_or_default();
        let member = inner.member(&assignee).map_err(|(_, m)| m)?;
        if held_stalled {
            return Err("task signer quota exceeded".to_owned());
        }
        let version = held.state.task().map_or(0, |t| t.version) + 1;
        let body = TaskEventBody::Update(TaskUpdate {
            task_id: task_id.to_owned(),
            version,
            state,
            steps,
            result_ref: result_ref.map(str::to_owned),
            result_summary: result_summary.map(str::to_owned),
        });
        inner.commit(conversation, task_id, &member, body).map_err(|(_, m)| m)
    }

    /// Another member `principal` starts a task for `assignee`.
    ///
    /// # Errors
    ///
    /// The protocol's refusal.
    pub fn start_as(
        &self,
        principal: &str,
        conversation: &str,
        task_id: &str,
        assignee: &str,
        text: &str,
    ) -> Result<u64, String> {
        let mut inner = self.inner();
        let member = inner.member(principal).map_err(|(_, m)| m)?;
        let body = TaskEventBody::Start(TaskStart {
            task_id: task_id.to_owned(),
            conversation_id: conversation.to_owned(),
            requester: member.principal.clone(),
            assignee: assignee.to_owned(),
            text: text.to_owned(),
            constraints: None,
        });
        inner.commit(conversation, task_id, &member, body).map_err(|(_, m)| m)
    }

    /// Another member `principal` accepts a proposal.
    ///
    /// # Errors
    ///
    /// The protocol's refusal.
    pub fn accept_as(
        &self,
        principal: &str,
        conversation: &str,
        task_id: &str,
    ) -> Result<u64, String> {
        let mut inner = self.inner();
        let member = inner.member(principal).map_err(|(_, m)| m)?;
        let version = inner.version(conversation, task_id).map_err(|(_, m)| m)? + 1;
        let body = TaskEventBody::Accept(TaskAccept { task_id: task_id.to_owned(), version });
        inner.commit(conversation, task_id, &member, body).map_err(|(_, m)| m)
    }

    /// The assignee used up its quota: its tasks show "stalled by
    /// assignee" and its updates are refused.
    pub fn stall(&self, conversation: &str, task_id: &str) {
        let mut inner = self.inner();
        if let Some(held) = inner.tasks.get_mut(&(conversation.to_owned(), task_id.to_owned())) {
            held.stalled = true;
        }
        let meta = json!({ "conversation_id": conversation, "task_id": task_id,
                           "stalled_by_assignee": true });
        inner.announce(meta);
    }

    /// Maintenance compacted the task's history (its view is kept),
    /// announced as `task.changed {compacted}` like the daemon does.
    pub fn compact(&self, conversation: &str, task_id: &str) {
        let mut inner = self.inner();
        let Some(held) = inner.tasks.get_mut(&(conversation.to_owned(), task_id.to_owned())) else {
            return;
        };
        held.compacted = true;
        let version = held.state.task().map_or(0, |t| t.version);
        inner.announce(json!({ "kind": "compacted", "conversation_id": conversation,
                               "task_id": task_id, "version": version, "compacted": true,
                               "removed": false }));
    }

    /// Maintenance pruned the task: the daemon no longer holds it
    /// (`task.get` answers null), announced as `task.changed {kind:
    /// removed}` like the daemon does.
    pub fn remove(&self, conversation: &str, task_id: &str) {
        let mut inner = self.inner();
        let Some(held) = inner.tasks.remove(&(conversation.to_owned(), task_id.to_owned())) else {
            return;
        };
        let version = held.state.task().map_or(0, |t| t.version);
        inner.announce(json!({ "kind": "removed", "conversation_id": conversation,
                               "task_id": task_id, "version": version, "compacted": false,
                               "removed": true }));
    }

    /// The requester's decision on the approval `approval_id` (the scoped
    /// id) of step `step` could not reach the assignee: its delivery is
    /// `failed` (`retry_exhausted`), announced like the daemon does.
    pub fn fail_delivery(&self, conversation: &str, task_id: &str, step: &str, approval_id: &str) {
        let mut inner = self.inner();
        let decision = inner.aliases.get(approval_id).cloned().unwrap_or_default();
        let approved = inner.decided.get(&decision).copied().unwrap_or(true);
        let delivery = Delivery {
            conversation: conversation.to_owned(),
            task: task_id.to_owned(),
            step: step.to_owned(),
            approved,
            state: "failed",
        };
        inner.deliveries.insert(approval_id.to_owned(), delivery.clone());
        inner.announce_delivery(approval_id, &delivery);
    }

    /// `grant.approve` / `grant.deny` on an approval whose decision was
    /// not delivered: the same decision is sent again (`Some(true)`), a
    /// conflicting one is refused (`Some(false)`); `None` when no
    /// decision of it failed to arrive.
    pub fn redeliver(&self, approval_id: &str, approved: bool) -> Option<bool> {
        let mut inner = self.inner();
        let delivery = inner.deliveries.get_mut(approval_id)?;
        if delivery.approved != approved {
            return Some(false);
        }
        delivery.state = "submitted";
        let delivery = delivery.clone();
        inner.announce_delivery(approval_id, &delivery);
        Some(true)
    }

    /// The requester's wait for the assignee ran out: pending resolutions
    /// stay pending (the assignee may still confirm them) as
    /// `executor_timeout`.
    pub fn time_out_decisions(&self) {
        let mut inner = self.inner();
        let mut timed_out = Vec::new();
        for ((conversation, task), held) in &mut inner.tasks {
            if held.pending_since.is_some() && !held.timed_out {
                held.timed_out = true;
                timed_out.push((conversation.clone(), task.clone()));
            }
        }
        for (conversation, task) in timed_out {
            inner.announce(json!({ "conversation_id": conversation, "task_id": task,
                                   "pending": true, "pending_state": "executor_timeout" }));
        }
    }

    /// The kind of `principal` is verified from now on (a root-signed
    /// profile states it and its current device signed the events).
    pub fn verify_kind(&self, principal: &str) {
        self.inner().verified.push(principal.to_owned());
    }

    /// The kind `principal`'s profile states (`human`, `headless_ai`,
    /// `bot`), reported as each actor row's `actor_kind`.
    pub fn set_kind(&self, principal: &str, kind: &'static str) {
        self.inner().kinds.insert(principal.to_owned(), kind);
    }

    /// The daemon's view of a task (`task.get`), or null.
    pub fn view(&self, conversation: &str, task_id: &str) -> Value {
        let inner = self.inner();
        inner
            .tasks
            .get(&(conversation.to_owned(), task_id.to_owned()))
            .map_or(Value::Null, |h| inner.view(h))
    }

    /// The task's current revision (0 when unknown).
    pub fn version(&self, conversation: &str, task_id: &str) -> u64 {
        self.view(conversation, task_id)["version"].as_u64().unwrap_or(0)
    }

    /// How many resolutions wait for the assignee.
    pub fn pending(&self) -> usize {
        self.inner().tasks.values().map(|h| h.pending.len()).sum()
    }

    /// Rewrites the last task message of `conversation` the way a sibling
    /// device of the account that sent it lists that event (seen on a live
    /// daemon: the second device of the account whose first device accepted
    /// a proposal): the row names its sender by principal, keeps the
    /// event's text rendering as `body_utf8`, and has no task fields
    /// (`content_type`, `task_id`, `actor_kind_verified`).
    pub fn as_sibling_copy(&self, conversation: &str) {
        let mut inner = self.inner();
        let members = inner.members.clone();
        let Some(row) = inner.messages.get_mut(conversation).and_then(|rows| rows.last_mut())
        else {
            return;
        };
        let device = row["sender_id"].as_str().unwrap_or_default().to_owned();
        if let Some(member) = members.iter().find(|m| m.device == device) {
            row["sender_id"] = json!(member.principal);
        }
        if let Some(map) = row.as_object_mut() {
            for field in ["content_type", "task_id", "actor_kind_verified"] {
                map.remove(field);
            }
        }
    }

    /// Adds a plain message row (text a member wrote) to `conversation`.
    pub fn push_message(&self, conversation: &str, sender: &str, text: &str) {
        let mut inner = self.inner();
        inner.clock += 1;
        inner.ids += 1;
        let (at, id) = (inner.clock, inner.ids);
        inner.messages.entry(conversation.to_owned()).or_default().push(json!({
            "conversation_id": conversation, "message_id": format!("plain-msg-{id}"),
            "sender_id": sender, "body_utf8": text, "encrypted_body": text.as_bytes(),
            "deleted": false, "created_at": at, "receipts": [],
        }));
    }

    /// The task messages of a conversation, as `message.list` rows.
    pub fn messages(&self, conversation: &str) -> Vec<Value> {
        let inner = self.inner();
        if inner.messages_held {
            return Vec::new();
        }
        inner.messages.get(conversation).cloned().unwrap_or_default()
    }

    /// Answers a `task.*` request of the local account; `None` for other
    /// methods.
    pub fn handle(&self, method: &str, body: &Value) -> Option<Reply> {
        if !method.starts_with("task.") {
            return None;
        }
        let mut inner = self.inner();
        if inner.refuse_next.as_ref().is_some_and(|(m, _)| m == method)
            && let Some((_, refusal)) = inner.refuse_next.take()
        {
            return Some(refused(refusal));
        }
        Some(match inner.request(method, body) {
            Ok(answer) => Reply::Ok(answer),
            Err(message) => refused(message),
        })
    }
}

fn step(id: &str, title: &str, state: TaskStatus, needs_me: Option<NeedsMe>) -> TaskStep {
    TaskStep { id: id.to_owned(), title: title.to_owned(), state, needs_me }
}

/// A step for [`FakeTasks::update`].
pub fn task_step(id: &str, title: &str, state: TaskStatus) -> TaskStep {
    step(id, title, state, None)
}

/// A step that waits on a choice in the assignee's surface `surface_id`
/// (the assignee's own id of it).
pub fn choice_step(id: &str, title: &str, surface_id: &str) -> TaskStep {
    let place = NeedsMe::Choice { surface_id: surface_id.to_owned() };
    step(id, title, TaskStatus::NeedsMe, Some(place))
}

/// A step that waits on an approval of the grant `grant_id`.
pub fn approval_step(id: &str, title: &str, grant_id: &str) -> TaskStep {
    step(id, title, TaskStatus::NeedsMe, Some(NeedsMe::Approval { grant_id: grant_id.to_owned() }))
}

fn apply(state: &mut TaskState, event: &TaskEvent, member: &Member) -> Result<u64, Refused> {
    let public = member.public();
    let signer = TaskSigner {
        principal_id: &member.principal,
        actor_type: member.kind.clone(),
        signing_key_id: &member.device,
        public_key_base64url: &public,
    };
    state.apply(event, &signer).map(|t| t.version).map_err(|e| rejected(&e))
}

fn meta(task: &ramflux_protocol::Task) -> Value {
    json!({
        "task_id": task.task_id, "conversation_id": task.conversation_id,
        "version": task.version, "state": task.state, "actor_kind_verified": false,
    })
}

/// A decision row as `task.get` `approval_decisions[]` lists it.
fn delivery_row(approval_id: &str, d: &Delivery) -> Value {
    json!({ "approval_id": approval_id, "decision_id": format!("mcpd_{approval_id}"),
            "grant_id": format!("grant_{approval_id}"),
            "task_binding": { "conversation_id": d.conversation, "task_id": d.task,
                              "step_id": d.step },
            "status": if d.approved { "approved" } else { "denied" },
            "delivery_state": d.state,
            "reason_code": (d.state == "failed").then_some("retry_exhausted"),
            "expires_at": 1_990_000_000 })
}

fn view(held: &Held, inner: &Inner) -> Value {
    let Some(task) = held.state.task() else { return Value::Null };
    let mut view = serde_json::to_value(task).unwrap_or(Value::Null);
    view["id"] = json!(task.task_id);
    view["actor_kind_verified"] = json!(false);
    // As the daemon's actor rows: the principal, the device of its last
    // event (empty when none is known), its profile kind and the flag.
    let actor = |principal: Option<&str>| {
        let device = principal
            .and_then(|p| inner.members.iter().find(|m| m.principal == p))
            .map_or("", |m| m.device.as_str());
        json!({ "principal_id": principal, "device_id": device,
                "actor_kind": principal.and_then(|p| inner.kinds.get(p)),
                "actor_kind_verified": principal.is_some_and(|p| inner.verified.iter().any(|v| v == p)) })
    };
    view["actors"] = json!({
        "requester": actor(task.requester.as_deref()),
        "assignee": actor(Some(&task.assignee)),
        "acceptor": actor(None),
    });
    view["pending_resolution"] =
        held.pending_since.as_ref().map_or(Value::Null, |(message, at, step)| {
            json!({ "task_id": task.task_id, "conversation_id": task.conversation_id,
                "step_id": step, "message_id": message, "executor_device_id": "device-bo",
                "queued_at": at,
                "state": if held.timed_out { "executor_timeout" } else { "awaiting_executor" } })
        });
    view["created_at"] = json!(held.created_at);
    view["updated_at"] = json!(held.updated_at);
    if held.stalled && !task.state.is_terminal() {
        view["stalled_by_assignee"] = json!(true);
        view["progress_state"] = json!("stalled by assignee");
    }
    if held.compacted {
        view["compacted"] = json!(true);
        view["certificate_hashes"] = json!([]);
    }
    view
}

impl Inner {
    fn view(&self, held: &Held) -> Value {
        let mut view = view(held, self);
        if let Some(task) = held.state.task() {
            let rows: Vec<Value> = self
                .deliveries
                .iter()
                .filter(|(_, d)| d.conversation == task.conversation_id && d.task == task.task_id)
                .map(|(id, d)| delivery_row(id, d))
                .collect();
            view["approval_decisions"] = json!(rows);
        }
        view
    }

    fn announce_delivery(&self, approval_id: &str, delivery: &Delivery) {
        let reason = (delivery.state == "failed").then_some("retry_exhausted");
        self.announce(json!({ "kind": "updated", "conversation_id": delivery.conversation,
                              "task_id": delivery.task, "step_id": delivery.step,
                              "approval_id": approval_id, "delivery_state": delivery.state,
                              "reason_code": reason }));
    }

    fn member(&self, principal: &str) -> Result<Member, Refused> {
        self.members
            .iter()
            .find(|m| m.principal == principal)
            .cloned()
            .ok_or_else(|| ("not_member", format!("not a member: {principal}")))
    }

    fn agent(&self) -> Result<Member, Refused> {
        self.members
            .iter()
            .find(|m| m.kind == ActorType::Bot)
            .cloned()
            .ok_or_else(|| invalid("no AI participant"))
    }

    fn local(&self) -> Result<Member, Refused> {
        self.local.clone().ok_or_else(|| ("no_local_role", "no local account".to_owned()))
    }

    fn announce(&self, meta: Value) {
        if self.quiet {
            return;
        }
        if let Some((publisher, account)) = &self.announce {
            publisher.publish(account, "task.changed", meta);
        }
    }

    /// Signs and applies `body` as `member`; records, announces and leaves
    /// its message. Nothing changes when the protocol refuses it.
    fn commit(
        &mut self,
        conversation: &str,
        task_id: &str,
        member: &Member,
        body: TaskEventBody,
    ) -> Result<u64, Refused> {
        let event = member.sign(conversation, body)?;
        let key = (conversation.to_owned(), task_id.to_owned());
        let mut next = self.tasks.get(&key).map(|h| h.state.clone()).unwrap_or_default();
        let version = apply(&mut next, &event, member)?;
        self.clock += 1;
        let held = self.tasks.entry(key).or_default();
        if held.created_at == 0 {
            held.created_at = self.clock;
        }
        held.state = next;
        self.changed(conversation, task_id, &event, member);
        Ok(version)
    }

    fn changed(&mut self, conversation: &str, task_id: &str, event: &TaskEvent, member: &Member) {
        self.clock += 1;
        self.ids += 1;
        let at = self.clock;
        let Some(held) = self.tasks.get_mut(&(conversation.to_owned(), task_id.to_owned())) else {
            return;
        };
        held.updated_at = at;
        let Some(task) = held.state.task() else { return };
        let meta = meta(task);
        // As the daemon records it: the unverified actor kind is named once,
        // after the rendering when the rendering does not name it itself.
        let mut fallback = event.fallback_text();
        if !fallback.contains("(actor kind unverified)") {
            fallback.push_str(" (actor kind unverified)");
        }
        let (kind, verified) = if self.event_kinds {
            (
                self.kinds.get(member.principal.as_str()).copied(),
                self.verified.contains(&member.principal),
            )
        } else {
            (None, false)
        };
        let mut row = json!({
            "conversation_id": conversation, "message_id": format!("task-msg-{}", self.ids),
            "sender_id": member.device, "content_type": "task", "task_id": task_id,
            "body_utf8": fallback,
            "encrypted_body": fallback.as_bytes(), "actor_kind_verified": verified,
            "deleted": false, "created_at": at, "receipts": [],
        });
        if self.event_kinds {
            row["actor_kind"] = json!(kind);
        }
        self.messages.entry(conversation.to_owned()).or_default().push(row);
        self.announce(meta);
        if self.message_events
            && !self.quiet
            && !self.messages_held
            && let Some((publisher, account)) = &self.announce
        {
            let message = format!("task-msg-{}", self.ids);
            publisher.publish(
                account,
                "message.received",
                json!({ "message": { "id": message, "conversation_id": conversation },
                        "conversation": { "id": conversation } }),
            );
        }
    }

    fn version(&self, conversation: &str, task_id: &str) -> Result<u64, Refused> {
        self.tasks
            .get(&(conversation.to_owned(), task_id.to_owned()))
            .and_then(|h| h.state.task())
            .map(|t| t.version)
            .ok_or_else(|| ("not_found", "task not found".to_owned()))
    }

    /// The conversation a request names, or the only one holding its task.
    fn conversation_of(&self, body: &Value) -> Result<String, Refused> {
        if let Some(conversation) = body["conversation_id"].as_str() {
            return Ok(conversation.to_owned());
        }
        let task = field(body, "task_id")?;
        let mut found = self.tasks.keys().filter(|(_, t)| t == task);
        match (found.next(), found.next()) {
            (Some((conversation, _)), None) => Ok(conversation.clone()),
            (Some(_), Some(_)) => Err(invalid("ambiguous task id; conversation_id is required")),
            _ => Err(("not_found", "task not found".to_owned())),
        }
    }

    fn request(&mut self, method: &str, body: &Value) -> Result<Value, Refused> {
        match method {
            "task.get" => {
                let conversation = self.conversation_of(body).unwrap_or_default();
                let task = field(body, "task_id")?;
                let held = self.tasks.get(&(conversation, task.to_owned()));
                Ok(held.map_or(Value::Null, |h| self.view(h)))
            }
            "task.list" => self.list(body),
            _ => self.author(method, body),
        }
    }

    fn list(&self, body: &Value) -> Result<Value, Refused> {
        let conversation = field(body, "conversation_id")?;
        let state = body["state"].as_str();
        let limit =
            usize::try_from(body["limit"].as_u64().unwrap_or(50).clamp(1, 100)).map_err(invalid)?;
        let skip: usize = body["cursor"].as_str().and_then(|c| c.parse().ok()).unwrap_or(0);
        let rows: Vec<Value> = self
            .tasks
            .iter()
            .filter(|((c, _), _)| c == conversation)
            .map(|(_, held)| self.view(held))
            .filter(|v| state.is_none_or(|s| v["state"] == s))
            .collect();
        let page: Vec<Value> = rows.iter().skip(skip).take(limit).cloned().collect();
        let next = (skip + page.len() < rows.len()).then(|| (skip + page.len()).to_string());
        Ok(json!({ "tasks": page, "next_cursor": next }))
    }

    /// A change the local person makes: signed by its key, preflighted on
    /// the protocol, then applied (a resolution waits for the assignee).
    fn author(&mut self, method: &str, body: &Value) -> Result<Value, Refused> {
        let local = self.local()?;
        let conversation = self.conversation_of(body)?;
        self.ids += 1;
        if body.get("proposal_id").is_some() {
            return Err(invalid("proposal acceptance uses task.accept {task_id,version}"));
        }
        let task_id =
            body["task_id"].as_str().map_or_else(|| format!("task-{}", self.ids), str::to_owned);
        let message = body["message_id"]
            .as_str()
            .map_or_else(|| format!("task-req-{}", self.ids), str::to_owned);
        let extras = ["conversation_id", "message_id"];
        let start_extras = ["message_id"];
        let event = match method {
            "task.start" => {
                let mut start = body.clone();
                start["requester"] = json!(local.principal);
                start["task_id"] = json!(task_id);
                TaskEventBody::Start(payload::<TaskStart>(&start, &start_extras)?)
            }
            "task.accept" => TaskEventBody::Accept(payload::<TaskAccept>(body, &extras)?),
            "task.decline" => TaskEventBody::Decline(payload::<TaskDecline>(body, &extras)?),
            "task.cancel" => {
                let mut cancel = body.clone();
                if let Some(version) = cancel.as_object_mut().and_then(|m| m.remove("version")) {
                    cancel["base_revision"] = version;
                }
                TaskEventBody::Cancel(payload::<TaskCancel>(&cancel, &extras)?)
            }
            "task.resolve" => {
                let mut resolve = body.clone();
                if let Some(decision) = self.aliases.get(text_of(body, "decision_id")) {
                    resolve["decision_id"] = json!(decision);
                }
                TaskEventBody::Resolve(payload::<TaskResolve>(&resolve, &extras)?)
            }
            _ => return Err(invalid("unsupported task method")),
        };
        let resolution = match &event {
            TaskEventBody::Resolve(r) => Some((r.decision_id.clone(), r.step_id.clone())),
            _ => None,
        };
        let answer = |pending: bool| {
            json!({ "accepted": !pending, "pending": pending, "task_id": task_id,
                    "message_id": message, "delivery_state": "submitted",
                    "actor_kind_verified": false })
        };
        if let Some((decision, step)) = resolution {
            let signed = local.sign(&conversation, event)?;
            let key = (conversation.clone(), task_id.clone());
            let held =
                self.tasks.get_mut(&key).ok_or(("not_found", "task not found".to_owned()))?;
            let mut preflight = held.state.clone();
            apply(&mut preflight, &signed, &local)?;
            held.pending.insert(decision, (signed, local));
            held.pending_since = Some((message.clone(), self.clock, step));
            held.timed_out = false;
            self.announce(json!({ "conversation_id": conversation, "task_id": task_id,
                                  "pending": true, "pending_state": "awaiting_executor" }));
            return Ok(answer(true));
        }
        self.commit(&conversation, &task_id, &local, event)?;
        Ok(answer(false))
    }
}
