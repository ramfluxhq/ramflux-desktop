// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A request lane: a small pool of local-bus connections serving one scope
//! (the daemon, or one account).
//!
//! `LocalBusClient` serves one request at a time per connection, so each
//! worker owns one connection and takes the next queued request when it is
//! free. Connections open lazily and are replaced after a transport error or
//! a timeout (a late answer must never be read as the next request's).
//! Lanes of different scopes share nothing, so a slow request of one account
//! never waits behind another account's.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ramflux_sdk::LocalBusClient;
use serde_json::Value;
use tokio::sync::{Mutex, mpsc, oneshot};

use super::{BusError, BusRequest, wipe};

struct Job {
    request: BusRequest,
    reply: oneshot::Sender<Result<Value, BusError>>,
}

/// The sending end of a lane. Cheap to clone; workers stop when every
/// clone is dropped.
#[derive(Clone)]
pub struct Lane {
    jobs: mpsc::UnboundedSender<Job>,
}

impl Lane {
    /// Starts `workers` connections to `socket` on the current tokio runtime.
    pub fn spawn(socket: PathBuf, workers: usize, timeout: Duration) -> Self {
        let (jobs, queue) = mpsc::unbounded_channel();
        let queue = Arc::new(Mutex::new(queue));
        for _ in 0..workers.max(1) {
            tokio::spawn(worker(socket.clone(), Arc::clone(&queue), timeout));
        }
        Self { jobs }
    }

    /// Sends one request and waits for its answer.
    pub async fn call(&self, request: BusRequest) -> Result<Value, BusError> {
        let (reply, answer) = oneshot::channel();
        self.jobs
            .send(Job { request, reply })
            .map_err(|_| BusError::Transport("request lane stopped".to_owned()))?;
        answer.await.unwrap_or_else(|_| Err(BusError::Transport("request dropped".to_owned())))
    }
}

async fn worker(
    socket: PathBuf,
    queue: Arc<Mutex<mpsc::UnboundedReceiver<Job>>>,
    timeout: Duration,
) {
    let mut client: Option<LocalBusClient> = None;
    loop {
        let job = { queue.lock().await.recv().await };
        let Some(job) = job else { break };
        let result = run(&mut client, &socket, &job.request, timeout).await;
        let _ = job.reply.send(result);
    }
}

async fn run(
    client: &mut Option<LocalBusClient>,
    socket: &PathBuf,
    request: &BusRequest,
    timeout: Duration,
) -> Result<Value, BusError> {
    if client.is_none() {
        let connected = tokio::time::timeout(timeout, LocalBusClient::connect(socket))
            .await
            .map_err(|_| BusError::Timeout)?
            .map_err(BusError::from)?;
        *client = Some(connected);
    }
    let Some(connection) = client.as_mut() else {
        return Err(BusError::Transport("no connection".to_owned()));
    };
    let body = request.wire_body();
    let answer = tokio::time::timeout(
        timeout,
        connection.request(request.account.clone(), request.sdk_api, &request.method, &body),
    )
    .await;
    wipe(body);
    match answer {
        Err(_) => {
            *client = None;
            Err(BusError::Timeout)
        }
        Ok(Err(error)) => {
            let error = BusError::from(error);
            if error.is_connection_lost() {
                *client = None;
            }
            Err(error)
        }
        Ok(Ok(value)) => Ok(value),
    }
}
