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
//! from its saved body or joins the live lifecycle execution. Requests without a key are
//! not resent here; read recovery, events and explicit actions own later attempts.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ramflux_sdk::{LocalBusClient, LocalBusFrame, LocalBusFrameKind};
use serde_json::Value;
use tokio::sync::{Mutex, Semaphore, mpsc};

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

#[derive(Debug, Clone, Copy)]
pub(crate) struct ConnectionState {
    pub(crate) generation: u64,
    pub(crate) connected: bool,
}

struct OpenConnection {
    generation: u64,
    client: LocalBusClient,
    observer: Option<tokio::task::AbortHandle>,
}

impl Drop for OpenConnection {
    fn drop(&mut self) {
        if let Some(observer) = &self.observer {
            observer.abort();
        }
    }
}

struct Inner {
    socket: PathBuf,
    connect_timeout: Duration,
    deadlines: Deadlines,
    /// The open connection and its generation.
    link: Mutex<Option<OpenConnection>>,
    status: Option<mpsc::UnboundedSender<ConnectionState>>,
    generations: AtomicU64,
    next_id: AtomicU64,
    slots: Semaphore,
    approval: super::approval::Approval,
}

impl BusClient {
    /// A client of `socket`; nothing connects until the first request.
    pub fn new(socket: PathBuf, connect_timeout: Duration, deadlines: Deadlines) -> Self {
        Self::with_status(socket, connect_timeout, deadlines, None)
    }

    pub(crate) fn with_status(
        socket: PathBuf,
        connect_timeout: Duration,
        deadlines: Deadlines,
        status: Option<mpsc::UnboundedSender<ConnectionState>>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                socket,
                connect_timeout,
                deadlines,
                link: Mutex::new(None),
                status,
                generations: AtomicU64::new(0),
                next_id: AtomicU64::new(1),
                slots: Semaphore::new(MAX_IN_FLIGHT),
                approval: super::approval::Approval::default(),
            }),
        }
    }

    /// The approval this client checks before transmitting message-class
    /// requests (not enforced until [`Approval::enforce`]).
    pub(crate) fn approval(&self) -> &super::approval::Approval {
        &self.inner.approval
    }

    /// Requests that hold a slot now: sent and not yet answered (tests).
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn in_flight(&self) -> usize {
        MAX_IN_FLIGHT - self.inner.slots.available_permits()
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
        match self.once(request).await.1 {
            Err(error) if request.operation_id.is_some() && answer_lost(&error) => {
                self.once(request).await.1
            }
            other => other,
        }
    }

    /// Like [`call`](Self::call) without the keyed resend, and with the
    /// generation of the connection the request travelled on (0 when none
    /// was opened).
    pub(crate) async fn call_on_connection(
        &self,
        request: &BusRequest,
    ) -> (u64, Result<Value, BusError>) {
        self.once(request).await
    }

    async fn once(&self, request: &BusRequest) -> (u64, Result<Value, BusError>) {
        let _slot = match self.inner.slots.acquire().await {
            Ok(slot) => slot,
            Err(_) => {
                return (0, Err(BusError::Transport("request connection closed".to_owned())));
            }
        };
        let (generation, client) = match self.connection().await {
            Ok(open) => open,
            Err(error) => return (0, Err(error)),
        };
        // The approval names one connection: a request that reconnected is
        // checked against the connection it actually got.
        if !self.inner.approval.permits(&request.method, generation) {
            return (
                generation,
                Err(BusError::Daemon {
                    code: super::approval::GATE_CLOSED.to_owned(),
                    message: "messages are not approved on this connection".to_owned(),
                    retry_after_ms: None,
                    reason_code: None,
                    details: None,
                    cause: None,
                }),
            );
        }
        (generation, self.exchange_once(request, generation, &client).await)
    }

    async fn exchange_once(
        &self,
        request: &BusRequest,
        generation: u64,
        client: &LocalBusClient,
    ) -> Result<Value, BusError> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let answer = exchange(
            client,
            request,
            self.inner.deadlines,
            &format!("desk_{id}"),
            &self.inner.socket,
            "ordinary",
        )
        .await;
        if answer.as_ref().is_err_and(BusError::is_connection_lost) {
            self.forget(generation).await;
        }
        answer
    }

    async fn connection(&self) -> Result<(u64, LocalBusClient), BusError> {
        let mut link = self.inner.link.lock().await;
        if let Some(open) = link
            .as_ref()
            .filter(|open| open.observer.as_ref().is_none_or(|task| !task.is_finished()))
        {
            return Ok((open.generation, open.client.clone()));
        }
        link.take();
        let client = tokio::time::timeout(
            self.inner.connect_timeout,
            LocalBusClient::connect(&self.inner.socket),
        )
        .await
        .map_err(|_| BusError::Transport("the daemon socket did not accept in time".to_owned()))?
        .map_err(BusError::from)?;
        let generation = self.inner.generations.fetch_add(1, Ordering::Relaxed) + 1;
        let observer = self.inner.status.as_ref().map(|out| {
            let out = out.clone();
            let _ = out.send(ConnectionState { generation, connected: true });
            let mut client = client.clone();
            tokio::spawn(async move {
                // Ordinary connections have no subscription; their event receiver
                // also closes on EOF, without a probe or a periodic request.
                while client.next_event().await.is_ok() {}
                let _ = out.send(ConnectionState { generation, connected: false });
            })
            .abort_handle()
        });
        *link = Some(OpenConnection { generation, client: client.clone(), observer });
        Ok((generation, client))
    }

    /// Drops the connection of `generation` if it is still the open one.
    async fn forget(&self, generation: u64) {
        let mut link = self.inner.link.lock().await;
        if link.as_ref().is_some_and(|open| open.generation == generation) {
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
    socket: &Path,
    connection_class: &str,
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
    super::trace::record(socket, connection_class, &frame);
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
