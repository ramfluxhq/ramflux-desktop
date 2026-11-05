// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Text sends: only a typed pre-acceptance refusal permits a fresh attempt.
use super::scope::{Internal, Task};
use crate::bus::{BusError, BusRequest, idempotency, wipe};
use crate::plan::fetch::Fetch;
use ramflux_desktop_core::{Receipt, Timestamp, methods};
use serde_json::Value;
use std::time::Duration;
use tokio::time::Instant;

pub(super) struct PendingSend {
    body: Value,
    request: BusRequest,
    refresh: Vec<Fetch>,
    started: Instant,
    at: Timestamp,
    refusals: usize,
    due: Option<Instant>,
}

impl Drop for PendingSend {
    fn drop(&mut self) {
        wipe(std::mem::take(&mut self.body));
        wipe(std::mem::take(&mut self.request.body));
    }
}

/// The error code identifies the refusal; details confirm its networking context.
fn pending(error: &BusError) -> Option<Option<u64>> {
    match error {
        BusError::Daemon { code, details: Some(details), retry_after_ms, .. }
            if code == "NetworkingPending" && details["networking"].is_object() =>
        {
            Some(*retry_after_ms)
        }
        _ => None,
    }
}

impl Task {
    pub(super) fn start_send(&mut self, body: Value, request: BusRequest, refresh: Vec<Fetch>) {
        let Some(id) = request.body["message_id"].as_str().map(str::to_owned) else { return };
        let at = super::now();
        let events = self.translator.outgoing_message(&body, &id, Receipt::Sending, at);
        self.emit(events);
        self.sends.insert(
            id.clone(),
            PendingSend {
                body,
                request,
                refresh,
                started: Instant::now(),
                at,
                refusals: 0,
                due: None,
            },
        );
        self.dispatch_send(&id);
    }

    fn dispatch_send(&self, id: &str) {
        let Some(send) = self.sends.get(id) else { return };
        let request = BusRequest::new(
            send.request.account.as_deref(),
            send.request.sdk_api,
            send.request.method.clone(),
            send.request.body.clone(),
        );
        let request = match &send.request.operation_id {
            Some(key) => request.with_operation_id(key.clone()),
            None => request,
        };
        let (client, internal, id) = (self.client.clone(), self.internal.clone(), id.to_owned());
        tokio::spawn(async move {
            let result = client.call(&request).await;
            wipe(request.body);
            let _ = internal.send(Internal::SendDone { id, result });
        });
    }

    pub(super) fn next_send_due(&self) -> Option<Instant> {
        self.sends.values().filter_map(|s| s.due).min()
    }

    pub(super) fn send_done(&mut self, id: &str, result: Result<Value, BusError>) {
        let Some(send) = self.sends.get_mut(id) else { return };
        if let Err(error) = &result
            && let Some(hint) = pending(error)
        {
            const LADDER: [u64; 5] = [2_000, 5_000, 15_000, 30_000, 60_000];
            let delay = hint.unwrap_or(LADDER[send.refusals.min(4)]).clamp(1, 600_000);
            send.refusals = send.refusals.saturating_add(1);
            let due = Instant::now() + Duration::from_millis(delay);
            send.due = Some(due);
            let retry_at = send.at.saturating_add_ms(
                u64::try_from(due.duration_since(send.started).as_millis()).unwrap_or(u64::MAX),
            );
            let events = self.translator.outgoing_message(
                &send.body,
                id,
                Receipt::WaitingForNode { retry_at },
                send.at,
            );
            self.emit(events);
            return;
        }
        let Some(send) = self.sends.remove(id) else { return };
        let receipt =
            if result.is_ok() { Receipt::Sent } else { Receipt::Failed { retryable: false } };
        let events = self.translator.outgoing_message(&send.body, id, receipt, send.at);
        self.emit(events);
        self.on_effect_done(methods::SEND_MESSAGE, &send.body, result, send.refresh.clone());
    }

    pub(super) fn retry_sends(&mut self, live: bool) {
        let now = Instant::now();
        let ids: Vec<_> = self
            .sends
            .iter()
            .filter(|(_, s)| s.due.is_some_and(|due| live || due <= now))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            let Some(send) = self.sends.get_mut(&id) else { continue };
            // Only entries waiting after a typed refusal can reach this path.
            let Ok(key) = idempotency::fresh_key() else { continue };
            send.request.operation_id = Some(key);
            send.due = None;
            let events =
                self.translator.outgoing_message(&send.body, &id, Receipt::Sending, send.at);
            self.emit(events);
            self.dispatch_send(&id);
        }
    }
}

#[cfg(test)]
#[path = "send_tests.rs"]
mod tests;
