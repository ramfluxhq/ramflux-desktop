// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Performing a scope's effects.
//!
//! Each effect is planned against the translator, then its requests run as
//! spawned futures on the scope's connection and the answer comes back to
//! the task, which translates it. A plan of several requests runs them in
//! order and stops at the first failure, reporting how many were applied.
//! Attachment jobs run step by step on their own (see `media`). A safety
//! verification reads the compared snapshot first and verifies exactly it.
//! Request bodies are wiped after sending. A node lookup
//! that macOS Local Network privacy may have blocked is repeated a moment
//! later (see [`node_lookup`]).

use std::time::Duration;

use ramflux_desktop_core::{DomainEvent, Effect, EffectFailure, FailureCode, methods};
use serde_json::{Value, json};

use super::now;
use super::scope::{Internal, Task};
use crate::bus::client::BusClient;
use crate::bus::{BusError, BusRequest, idempotency, wipe};
use crate::capabilities;
use crate::plan::fetch::Fetch;
use crate::plan::{Plan, plan};
use crate::translate::node_failures;
use crate::translate::reason::{key, keyed};

impl Task {
    pub(super) fn effect(&mut self, effect: Effect) {
        // Creation and recovery ceremonies run on a connection of their own
        // (the daemon ties a ceremony to its connection), never through the
        // multiplexed request connection.
        let effect = match super::custody::Step::of(effect) {
            Ok(step) => {
                let context = super::custody::Context {
                    #[cfg(feature = "test-support")]
                    phrase_handoff_gate: self.config.phrase_handoff_gate.clone(),
                    socket: self.config.socket_path.clone(),
                    budget: self.config.deadlines.network,
                    internal: self.internal.clone(),
                    phrases: self.ceremonies.phrases.clone(),
                    audit: self.ceremonies.audit.clone(),
                };
                self.custody.send(step, &context);
                return;
            }
            Err(effect) => effect,
        };
        if let Some(method) = effect.method().filter(|m| super::gate::is_message_method(m))
            && !self.gate.is_open()
        {
            let failure = EffectFailure {
                method: method.to_owned(),
                code: FailureCode::Daemon("SchemaGateClosed".to_owned()),
                message: key::SCHEMA_GATE_CLOSED.to_owned(),
            };
            self.emit(vec![DomainEvent::EffectFailed(failure)]);
            return;
        }
        if !self.found.calls
            && let Some(method) = effect.method().filter(|m| capabilities::CALL_METHODS.contains(m))
        {
            let failure = EffectFailure {
                method: method.to_owned(),
                code: FailureCode::UnsupportedByBackend,
                message: capabilities::reason::CALLS.to_owned(),
            };
            self.emit(vec![DomainEvent::EffectFailed(failure)]);
            return;
        }
        if self.found.requests_missing
            && let Some(method) =
                effect.method().filter(|m| capabilities::REQUEST_METHODS.contains(m))
        {
            let failure = EffectFailure {
                method: method.to_owned(),
                code: FailureCode::UnsupportedByBackend,
                message: capabilities::reason::CONTACT_REQUESTS.to_owned(),
            };
            self.emit(vec![DomainEvent::EffectFailed(failure)]);
            return;
        }
        if !self.found.backup
            && let Some(method) =
                effect.method().filter(|m| capabilities::BACKUP_METHODS.contains(m))
        {
            let failure = EffectFailure {
                method: method.to_owned(),
                code: FailureCode::UnsupportedByBackend,
                message: capabilities::reason::FULL_BACKUP.to_owned(),
            };
            self.emit(vec![DomainEvent::EffectFailed(failure)]);
            return;
        }
        let planned = plan(self.scope.account(), effect, &self.translator);
        let (method, body) = (planned.method.clone(), planned.body.clone());
        self.translator.decision_sent(&method, &body);
        if method == methods::GROUP_CREATE && matches!(planned.plan, Plan::Calls { .. }) {
            // Until the member adds finish, the group takes no messages. A
            // refused creation runs nothing, so it never starts one.
            let context_id =
                ramflux_desktop_core::ContextId::new(crate::plan::text(&body, "context_id"));
            self.emit(vec![DomainEvent::GroupCreation { context_id, creating: true }]);
        }
        self.translator.rename_sent(&method, &body);
        let operation = match planned.plan {
            Plan::Call { request, refresh } if method == methods::SEND_MESSAGE => {
                if let Some(id) = body["retry_moment"].as_str() {
                    self.emit(vec![DomainEvent::MomentUpdated {
                        moment_id: ramflux_desktop_core::MomentId::new(id),
                        lifecycle: Some(ramflux_desktop_core::Lifecycle::Failed),
                        kind: Some(ramflux_desktop_core::MomentKind::Message(
                            ramflux_desktop_core::MessagePayload {
                                text: body["text"].as_str().unwrap_or_default().to_owned(),
                                receipt: ramflux_desktop_core::Receipt::Failed { retryable: false },
                                tombstone: false,
                                attachment: None,
                                franking: None,
                                ..Default::default()
                            },
                        )),
                    }]);
                }
                self.start_send(body, request, refresh);
                return;
            }
            other => other,
        };
        match operation {
            Plan::Local => {
                let events = self.translator.local(&method, &body);
                self.persist_pins();
                self.emit(events);
            }
            Plan::Refuse { code, message } => {
                self.emit(vec![DomainEvent::EffectFailed(EffectFailure { method, code, message })]);
            }
            Plan::Media(job) => self.media(job),
            other => {
                let client = self.client.clone();
                let job_link = self.job_link.clone();
                let internal = self.internal.clone();
                let (socket, connect_timeout, deadlines) = (
                    self.config.socket_path.clone(),
                    self.config.connect_timeout,
                    self.config.deadlines,
                );
                let retries: Vec<Duration> = self
                    .config
                    .local_network_retry_ms
                    .iter()
                    .take_while(|ms| **ms > 0)
                    .map(|ms| Duration::from_millis(u64::from(*ms)))
                    .collect();
                tokio::spawn(async move {
                    let (result, refresh) = match other {
                        // Taken only from the attended job connection.
                        Plan::Call { request, refresh }
                            if crate::bus::jobs::REQUESTS.contains(&request.method.as_str()) =>
                        {
                            (job_link.call(&request).await, refresh)
                        }
                        // Taken only from the owner's attended session, on a
                        // connection of its own.
                        Plan::Call { request, refresh }
                            if crate::bus::attended::is_attended(&request.method) =>
                        {
                            let result = crate::bus::attended::call(
                                &socket,
                                connect_timeout,
                                deadlines,
                                &request,
                            )
                            .await;
                            wipe(request.body);
                            (result, refresh)
                        }
                        other => perform(&client, other, &retries).await,
                    };
                    let _ = internal.send(Internal::EffectDone { method, body, result, refresh });
                });
            }
        }
    }

    pub(super) fn on_effect_done(
        &mut self,
        method: &str,
        body: &Value,
        result: Result<Value, BusError>,
        refresh: Vec<Fetch>,
    ) {
        if method.starts_with("account.") {
            self.refresh_daemon();
        }
        match result {
            Ok(answer) => {
                let mut events = self.translator.effect_done(method, body, &answer, now());
                events.push(DomainEvent::EffectSucceeded { method: method.to_owned() });
                if matches!(
                    method,
                    ramflux_desktop_core::methods::APP_LOCK_STATE
                        | ramflux_desktop_core::methods::APP_LOCK_CONFIGURE
                        | ramflux_desktop_core::methods::APP_LOCK_LIFECYCLE
                ) && let Some(id) = body["account_id"].as_str()
                {
                    self.app_lock_watch.observe(
                        &ramflux_desktop_core::AccountId::new(id),
                        &answer,
                        tokio::time::Instant::now(),
                    );
                }
                events.extend(Self::creation_ended(method, body));
                self.emit(events);
                self.spawn_fetches(refresh);
                self.resolve_task_step(method, body, &answer);
                if method == ramflux_desktop_core::methods::ACCOUNT_LOCK
                    && self.scope.account().is_some()
                {
                    self.locked();
                }
            }
            Err(error) => {
                // The app lock holds the account closed: the lock screen takes
                // over, and the refusal is no failure to report.
                if let BusError::Daemon { code: refusal, .. } = &error
                    && refusal == "app_locked"
                {
                    // The daemon session takes the account from the request.
                    let account = self.scope.account().cloned().or_else(|| {
                        body["account_id"].as_str().map(ramflux_desktop_core::AccountId::new)
                    });
                    self.app_lock_refused(account);
                    return;
                }
                // A lifecycle the daemon did not answer leaves the account's lock
                // unknown: it stays closed until the account code opens it.
                if method == ramflux_desktop_core::methods::APP_LOCK_LIFECYCLE {
                    if let Some(id) = body["account_id"].as_str() {
                        let account = ramflux_desktop_core::AccountId::new(id);
                        self.app_lock_watch.unreadable(
                            &account,
                            self.config.reconnect_backoff,
                            tokio::time::Instant::now(),
                        );
                        let events = self.translator.app_lock_unreadable(&account);
                        self.emit(events);
                    }
                    if error.is_connection_lost() {
                        self.lost();
                    }
                    return;
                }
                let code = match &error {
                    BusError::Daemon { code, .. } => FailureCode::Daemon(code.clone()),
                    BusError::Transport(_) => FailureCode::Transport,
                    BusError::Timeout => FailureCode::Timeout,
                };
                // A refused configure is shown with the daemon's reason code.
                // An unlock the daemon refuses for the account code is marked with
                // its reason (`invalid_argument`); other refusals keep their text.
                let message = match &error {
                    BusError::Daemon { reason_code: Some(reason), .. }
                        if method == ramflux_desktop_core::methods::APP_LOCK_CONFIGURE
                            || (method == ramflux_desktop_core::methods::ACCOUNT_UNLOCK
                                && reason == "invalid_argument") =>
                    {
                        reason.clone()
                    }
                    _ => self
                        .translator
                        .request_failure(method, body, &error)
                        .unwrap_or_else(|| self.translator.failure_message(method, &error)),
                };
                let closed = self.translator.effect_failed(method, body, &code, &message, now());
                // A refused update request is answered in its surface, next
                // to the button, and a refused task request on its task, next
                // to the controls that sent it; neither is a status-strip
                // error as well.
                let on_task = self.translator.task_refused(method, body, &code, &message);
                let mut events = if method == ramflux_desktop_core::methods::SURFACE_REQUEST_RESYNC
                {
                    Vec::new()
                } else if on_task.is_empty() {
                    let failure = EffectFailure { method: method.to_owned(), code, message };
                    vec![DomainEvent::EffectFailed(failure)]
                } else {
                    on_task
                };
                if self.scope.account().is_none()
                    && let Some(id) = body["local_account_id"].as_str()
                {
                    events.extend(Self::networking_failures(
                        &events,
                        &ramflux_desktop_core::AccountId::new(id),
                    ));
                }
                events.extend(closed);
                events.extend(Self::creation_ended(method, body));
                self.emit(events);
                // What applied before a failure is read back.
                self.spawn_fetches(refresh);
                if error.is_connection_lost() {
                    self.lost();
                }
            }
        }
    }
}

impl Task {
    /// The end of a group creation (success or failure) for `method`.
    fn creation_ended(method: &str, body: &Value) -> Option<DomainEvent> {
        (method == methods::GROUP_CREATE).then(|| DomainEvent::GroupCreation {
            context_id: ramflux_desktop_core::ContextId::new(crate::plan::text(body, "context_id")),
            creating: false,
        })
    }

    /// After the grant flow decided an approval a task step waits on, sends
    /// the requester's resolution of that step (the task records the
    /// decision; the grant alone authorizes the tool). It names the
    /// decision id the grant flow answered (the original MCP decision);
    /// the approval id when the answer names none (the daemon accepts a
    /// scoped approval id and canonicalizes it).
    fn resolve_task_step(&mut self, method: &str, body: &Value, answer: &Value) {
        if body["retry"] == true {
            // The same decision sent again: the step was resolved with it.
            return;
        }
        let approved = match method {
            ramflux_desktop_core::methods::GRANT_APPROVE => true,
            ramflux_desktop_core::methods::GRANT_DENY => false,
            _ => return,
        };
        let grant = ramflux_desktop_core::GrantId::new(crate::plan::text(body, "grant_id"));
        let decision = answer["decision_id"].as_str().filter(|d| !d.is_empty());
        if let Some(resolution) = self.translator.task_resolution(&grant, approved, decision) {
            self.effect(Effect::Call {
                method: ramflux_desktop_core::methods::TASK_RESOLVE.to_owned(),
                body: resolution,
            });
        }
    }
}

/// Performs a plan that talks to the daemon; returns its result and the
/// reads that follow it.
async fn perform(
    client: &BusClient,
    plan: Plan,
    local_network_retries: &[Duration],
) -> (Result<Value, BusError>, Vec<Fetch>) {
    match plan {
        Plan::Call { request, refresh } if is_node_lookup(&request.method) => {
            (node_lookup(client, request, local_network_retries).await, refresh)
        }
        Plan::Call { request, refresh } => {
            let result = client.call(&request).await;
            wipe(request.body);
            (result, refresh)
        }
        Plan::Calls { requests, refresh } => (in_order(client, requests).await, refresh),
        Plan::Verify { lookup, refresh } => (verify(client, lookup).await, refresh),
        Plan::Diagnostics { path } => (write_diagnostics(client, &path).await, Vec::new()),
        Plan::Local | Plan::Refuse { .. } | Plan::Media(_) => (Ok(Value::Null), Vec::new()),
    }
}

fn is_node_lookup(method: &str) -> bool {
    matches!(method, methods::NODE_ADD | methods::NODE_PROBE)
}

/// A node lookup, repeated after each of `retries` while it fails the way
/// a macOS Local Network privacy block does (see
/// [`node_failures::may_be_local_network_block`]): the blocked attempt is
/// what makes the system refresh its record of the app or show its prompt,
/// so the next one goes through. A repeat is a new attempt, so a mutating
/// lookup gets a fresh retry key (the failed one is never replayed). A node
/// lookup carries no secrets.
async fn node_lookup(
    client: &BusClient,
    request: BusRequest,
    retries: &[Duration],
) -> Result<Value, BusError> {
    let mut result = client.call(&request).await;
    for wait in retries {
        match &result {
            Err(error) if node_failures::may_be_local_network_block(&error.message()) => {}
            _ => break,
        }
        tokio::time::sleep(*wait).await;
        let mut again = BusRequest::new(
            request.account.as_deref(),
            request.sdk_api,
            request.method.clone(),
            request.body.clone(),
        );
        if request.operation_id.is_some() {
            match idempotency::fresh_key() {
                Ok(key) => again = again.with_operation_id(key),
                Err(_) => break,
            }
        }
        result = client.call(&again).await;
    }
    wipe(request.body);
    result
}

/// Runs `requests` in order; the first failure stops the rest. A failure
/// after some applied says how many.
async fn in_order(client: &BusClient, requests: Vec<BusRequest>) -> Result<Value, BusError> {
    let total = requests.len();
    let mut last = Value::Null;
    for (done, request) in requests.into_iter().enumerate() {
        let result = client.call(&request).await;
        wipe(request.body);
        match result {
            Ok(answer) => last = answer,
            Err(BusError::Daemon {
                code,
                message,
                reason_code,
                details,
                cause,
                retry_after_ms,
            }) if done > 0 => {
                let (done, total) = (done.to_string(), total.to_string());
                let message = keyed(
                    key::PARTIAL,
                    &[("done", &done), ("total", &total), ("reason", &message)],
                );
                return Err(BusError::Daemon {
                    code,
                    message,
                    reason_code,
                    details,
                    cause,
                    retry_after_ms,
                });
            }
            Err(error) => return Err(error),
        }
    }
    Ok(last)
}

/// `contact.safety_number`, then `contact.verify` with the compared
/// snapshot it answered (`expected`), under a fresh retry key.
async fn verify(client: &BusClient, lookup: BusRequest) -> Result<Value, BusError> {
    let safety = client.call(&lookup).await?;
    let Some(expected) = safety.get("expected").filter(|e| e.is_object()).cloned() else {
        return Err(BusError::daemon(
            "expected_safety_required",
            "the daemon answered no safety snapshot to compare",
        ));
    };
    let key = idempotency::fresh_key().map_err(|e| BusError::daemon("random", e))?;
    let request = BusRequest::new(
        lookup.account.as_deref(),
        "contact",
        "contact.verify",
        json!({ "expected": expected }),
    )
    .with_operation_id(key);
    client.call(&request).await
}

async fn write_diagnostics(client: &BusClient, path: &str) -> Result<Value, BusError> {
    let status = client.call(&Fetch::DaemonStatus.request(None)).await?;
    let nodes = client.call(&Fetch::NodeList.request(None)).await?;
    let text = crate::diagnostics::bundle(&status, &nodes);
    std::fs::write(path, text).map_err(|error| BusError::daemon("io", error.to_string()))?;
    Ok(json!({ "path": path }))
}

impl Task {
    /// What a creation or recovery step came to: the account it opened is
    /// recorded first (so the account list the translator keeps agrees with
    /// the ceremony's end), then the ceremony's events.
    pub(super) fn on_custody(&mut self, outcome: super::custody::Outcome) {
        let super::custody::Outcome { events, account, refresh } = outcome;
        if let Some(opened) = &account
            && let Some(custodian) = &opened.custodian
        {
            self.custodians.remember(opened.id.clone(), custodian.clone());
        }
        let mut all = account
            .map(|opened| self.translator.native_account_opened(&opened))
            .unwrap_or_default();
        all.extend(events);
        self.emit(all);
        self.spawn_fetches(refresh);
    }
}
