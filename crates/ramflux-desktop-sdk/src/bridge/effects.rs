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
//! may carry an attachment's bytes) are wiped after sending.

use ramflux_desktop_core::{DomainEvent, Effect, EffectFailure, FailureCode};
use serde_json::{Value, json};

use super::now;
use super::scope::{Internal, Link, Task};
use crate::bus::client::BusClient;
use crate::bus::{BusError, BusRequest, wipe};
use crate::plan::fetch::Fetch;
use crate::plan::{Plan, plan};
use crate::translate::reason::{key, keyed};

impl Task {
    pub(super) fn effect(&mut self, effect: Effect) {
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
                tokio::spawn(async move {
                    let (result, refresh) = perform(&client, other).await;
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
                let message = self.translator.failure_message(method, &error);
                let closed = self.translator.effect_failed(method, body, &code, &message, now());
                // A refused update request is answered in its surface, next
                // to the button; it is not a status-strip error as well.
                let mut events = if method == ramflux_desktop_core::methods::SURFACE_REQUEST_RESYNC
                {
                    Vec::new()
                } else {
                    let failure = EffectFailure { method: method.to_owned(), code, message };
                    vec![DomainEvent::EffectFailed(failure)]
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

/// Performs a plan that talks to the daemon; returns its result and the
/// reads that follow it.
async fn perform(client: &BusClient, plan: Plan) -> (Result<Value, BusError>, Vec<Fetch>) {
    match plan {
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
