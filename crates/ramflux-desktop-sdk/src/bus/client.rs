// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! One multiplexed request connection for one scope (the daemon, or one
//! account).
//!
//! The SDK's `LocalBusClient` keeps many requests in flight on one
//! connection and hands each answer to its request by request id, so a
//! scope needs one connection, not a pool. The connection opens lazily and
//! is replaced after it broke; a request that missed its deadline leaves it
//! open (a late answer is dropped by its id, never read as another
//! request's).
//!
//! Every frame carries its deadline and, when it changes state, its retry
//! key. A keyed request whose answer was lost (broken connection, missed
//! deadline) is sent once more with the same key: the daemon then answers
//! from its receipt instead of applying it twice. Requests without a key are
//! not resent here; reads are simply repeated by the next poll.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ramflux_sdk::{LocalBusClient, LocalBusFrame, LocalBusFrameKind};
use serde_json::Value;
use tokio::sync::{Mutex, Semaphore};

use super::deadline::{Deadlines, deadline_ms};
use super::{BusError, BusRequest};

/// Requests one connection keeps in flight (the SDK client refuses more).
pub const MAX_IN_FLIGHT: usize = 64;

/// Extra wait past a deadline before the client gives up by itself (the SDK
/// client already stops at the deadline; this only bounds a client that
/// never resolves).
const DEADLINE_GRACE: Duration = Duration::from_millis(500);

/// The request connection of one scope. Cheap to clone.
#[derive(Clone)]
pub struct BusClient {
    inner: Arc<Inner>,
}

struct Inner {
    socket: PathBuf,
    connect_timeout: Duration,
    deadlines: Deadlines,
    /// The open connection and its generation.
    link: Mutex<Option<(u64, LocalBusClient)>>,
    generations: AtomicU64,
    next_id: AtomicU64,
    slots: Semaphore,
}

impl BusClient {
    /// A client of `socket`; nothing connects until the first request.
    pub fn new(socket: PathBuf, connect_timeout: Duration, deadlines: Deadlines) -> Self {
        Self {
            inner: Arc::new(Inner {
                socket,
                connect_timeout,
                deadlines,
                link: Mutex::new(None),
                generations: AtomicU64::new(0),
                next_id: AtomicU64::new(1),
                slots: Semaphore::new(MAX_IN_FLIGHT),
            }),
        }
    }

    /// The budgets this client puts on its requests.
    pub fn deadlines(&self) -> Deadlines {
        self.inner.deadlines
    }

    /// How many connections this client has opened so far (one, unless a
    /// connection broke).
    pub fn connections_opened(&self) -> u64 {
        self.inner.generations.load(Ordering::Relaxed)
    }

    /// Sends one request and waits for its answer, at most until its
    /// deadline (twice, when a keyed request is resent).
    pub async fn call(&self, request: &BusRequest) -> Result<Value, BusError> {
        match self.once(request).await {
            Err(error) if request.operation_id.is_some() && answer_lost(&error) => {
                self.once(request).await
            }
            other => other,
        }
    }

    async fn once(&self, request: &BusRequest) -> Result<Value, BusError> {
        let _slot = self
            .inner
            .slots
            .acquire()
            .await
            .map_err(|_| BusError::Transport("request connection closed".to_owned()))?;
        let (generation, client) = self.connection().await?;
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let answer = exchange(&client, request, self.inner.deadlines, &format!("desk_{id}")).await;
        if answer.as_ref().is_err_and(BusError::is_connection_lost) {
            self.forget(generation).await;
        }
        answer
    }

    async fn connection(&self) -> Result<(u64, LocalBusClient), BusError> {
        let mut link = self.inner.link.lock().await;
        if let Some((generation, client)) = link.as_ref() {
            return Ok((*generation, client.clone()));
        }
        let client = tokio::time::timeout(
            self.inner.connect_timeout,
            LocalBusClient::connect(&self.inner.socket),
        )
        .await
        .map_err(|_| BusError::Transport("the daemon socket did not accept in time".to_owned()))?
        .map_err(BusError::from)?;
        let generation = self.inner.generations.fetch_add(1, Ordering::Relaxed) + 1;
        *link = Some((generation, client.clone()));
        Ok((generation, client))
    }

    /// Drops the connection of `generation` if it is still the open one.
    async fn forget(&self, generation: u64) {
        let mut link = self.inner.link.lock().await;
        if link.as_ref().is_some_and(|(open, _)| *open == generation) {
            *link = None;
        }
    }
}

/// Sends `request` as frame `request_id` on `client` and waits for its
/// answer, at most until its deadline.
pub(super) async fn exchange(
    client: &LocalBusClient,
    request: &BusRequest,
    deadlines: Deadlines,
    request_id: &str,
) -> Result<Value, BusError> {
    let budget = deadlines.budget(&request.method);
    // The secret-bearing body moves into the SDK client, which encodes and
    // drops it.
    let mut frame = LocalBusFrame::request(
        request_id,
        request.account.clone(),
        request.sdk_api,
        request.method.clone(),
        request.wire_body(),
    );
    frame.deadline_ms = Some(deadline_ms(budget));
    frame.operation_id.clone_from(&request.operation_id);
    // The whole response frame: a refusal keeps its typed reason and
    // details, which the convenience `request_frame` flattens to text.
    tokio::time::timeout(budget + DEADLINE_GRACE, client.request_frame_response(frame))
        .await
        .unwrap_or_else(|_| {
            Err(ramflux_sdk::SdkError::LocalBus("request deadline exceeded".to_owned()))
        })
        .map_err(BusError::from)
        .and_then(|response| match response.kind {
            LocalBusFrameKind::Error => Err(BusError::from_error_body(response.error)),
            _ => Ok(response.body),
        })
}

/// Whether the answer may have been lost after the daemon applied the
/// request.
fn answer_lost(error: &BusError) -> bool {
    matches!(error, BusError::Transport(_) | BusError::Timeout)
}
