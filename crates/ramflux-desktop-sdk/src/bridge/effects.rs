// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Performing a scope's effects.
//!
//! Each effect is planned against the translator, then its requests run as
//! spawned futures on the scope's connection and the answer comes back to
//! the task, which translates it. A plan of several requests runs them in
//! order and stops at the first failure, reporting how many were applied.
//! An object read (`Plan::Open`) writes the bytes where the user chose and
//! wipes them; only the path reaches the translator. Request bodies (which
//! may carry an attachment's bytes) are wiped after sending. A node lookup
//! that macOS Local Network privacy may have blocked is repeated a moment
//! later (see [`node_lookup`]).

use std::time::Duration;

use ramflux_desktop_core::{DomainEvent, Effect, EffectFailure, FailureCode, methods};
use serde_json::{Value, json};

use super::now;
use super::scope::{Internal, Link, Task};
use crate::bus::client::BusClient;
use crate::bus::{BusError, BusRequest, idempotency, wipe};
use crate::capabilities;
use crate::plan::fetch::Fetch;
use crate::plan::{Plan, plan};
use crate::translate::node_failures;
use crate::translate::reason::{key, keyed};

impl Task {
    pub(super) fn effect(&mut self, effect: Effect) {
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
        if !self.found.requests
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
        let planned = plan(self.scope.account(), effect, &self.translator);
        let (method, mut body) = (planned.method, planned.body);
        self.translator.decision_sent(&method, &body);
        self.translator.rename_sent(&method, &body);
        if method == ramflux_desktop_core::methods::ATTACH_OBJECT
            && let Plan::Call { request, .. } = &planned.plan
        {
            let events = self.translator.attachment_sending(&request.body, &mut body, now());
            self.emit(events);
        }
        match planned.plan {
            Plan::Local => {
                let events = self.translator.local(&method, &body);
                self.persist_pins();
                self.emit(events);
            }
            Plan::Refuse { code, message } => {
                self.emit(vec![DomainEvent::EffectFailed(EffectFailure { method, code, message })]);
            }
            other => {
                let client = self.client.clone();
                let internal = self.internal.clone();
                let retries: Vec<Duration> = self
                    .config
                    .local_network_retry_ms
                    .iter()
                    .take_while(|ms| **ms > 0)
                    .map(|ms| Duration::from_millis(u64::from(*ms)))
                    .collect();
                tokio::spawn(async move {
                    let (result, refresh) = perform(&client, other, &retries).await;
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
        match result {
            Ok(answer) => {
                let events = self.translator.effect_done(method, body, &answer, now());
                self.emit(events);
                self.spawn_fetches(refresh);
                self.resolve_task_step(method, body, &answer);
                self.interval = self.config.poll_fast;
                if method == ramflux_desktop_core::methods::ACCOUNT_LOCK
                    && self.scope.account().is_some()
                {
                    self.link = Link::Locked;
                }
            }
            Err(error) => {
                let code = match &error {
                    BusError::Daemon { code, .. } => FailureCode::Daemon(code.clone()),
                    BusError::Transport(_) => FailureCode::Transport,
                    BusError::Timeout => FailureCode::Timeout,
                };
                let message = self
                    .translator
                    .request_failure(method, body, &error)
                    .unwrap_or_else(|| self.translator.failure_message(method, &error));
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
                events.extend(closed);
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
        Plan::Open { request, save_to } => {
            (open(client, &request, save_to.as_deref()).await, Vec::new())
        }
        Plan::Diagnostics { path } => (write_diagnostics(client, &path).await, Vec::new()),
        Plan::Local | Plan::Refuse { .. } => (Ok(Value::Null), Vec::new()),
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
            Err(BusError::Daemon { code, message, reason_code, details }) if done > 0 => {
                let (done, total) = (done.to_string(), total.to_string());
                let message = keyed(
                    key::PARTIAL,
                    &[("done", &done), ("total", &total), ("reason", &message)],
                );
                return Err(BusError::Daemon { code, message, reason_code, details });
            }
            Err(error) => return Err(error),
        }
    }
    Ok(last)
}

/// `object.get`, then the bytes written to `save_to` (when set) and wiped.
async fn open(
    client: &BusClient,
    request: &BusRequest,
    save_to: Option<&str>,
) -> Result<Value, BusError> {
    let object = request.body["object_id"].clone();
    let mut answer = client.call(request).await?;
    let written = match (save_to, answer["plaintext_base64"].as_str()) {
        (Some(path), Some(encoded)) => {
            let bytes = ramflux_protocol::decode_base64url(encoded)
                .map(zeroize::Zeroizing::new)
                .map_err(|error| file_error(&error.to_string()));
            bytes.and_then(|bytes| {
                std::fs::write(path, bytes.as_slice()).map_err(|e| file_error(&e.to_string()))
            })
        }
        (Some(_), None) => Err(file_error("the daemon returned no object bytes")),
        (None, _) => Ok(()),
    };
    if let Some(Value::String(encoded)) = answer.get_mut("plaintext_base64") {
        zeroize::Zeroize::zeroize(encoded);
    }
    wipe(answer);
    written.map(|()| json!({ "object_id": object, "saved_to": save_to }))
}

fn file_error(reason: &str) -> BusError {
    BusError::daemon("io", keyed(key::FILE, &[("reason", reason)]))
}

async fn write_diagnostics(client: &BusClient, path: &str) -> Result<Value, BusError> {
    let status = client.call(&Fetch::DaemonStatus.request(None)).await?;
    let nodes = client.call(&Fetch::NodeList.request(None)).await?;
    let text = crate::diagnostics::bundle(&status, &nodes);
    std::fs::write(path, text).map_err(|error| BusError::daemon("io", error.to_string()))?;
    Ok(json!({ "path": path }))
}
