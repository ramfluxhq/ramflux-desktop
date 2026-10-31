// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The daemon-level backup job stream, and the connection job requests
//! travel on (BACKUP-01, product `docs/account_backup.md`).
//!
//! Backup and restore jobs belong to the daemon, not to an account: their
//! changes (`account.backup.job.changed`) arrive only on a subscription
//! without an account whose topics are exactly that one, opened with
//! `attended_frontend: true`. The daemon takes the job requests (create,
//! verify, install, cancel) only on a connection that opened such a
//! subscription: attendance belongs to the connection. So one connection
//! carries both: it subscribes first, then every job request is sent on it.
//!
//! The frame `seq` of a job event is the daemon's job stream cursor (each
//! row has its own per-job `seq` as well). The subscribe answer's
//! `head_seq` seeds the cursor, so a job whose events this connection never
//! saw is covered too. A connection opened again subscribes from the last
//! cursor; a cursor the daemon no longer covers (`resync_required`: it
//! restarted) subscribes from the head and asks its owner to read the jobs
//! again. A stream that fell
//! behind ends with `stream.lagged`, and a broken connection ends it: the
//! owner closes the link and opens it again.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ramflux_sdk::{LocalBusClient, LocalBusFrame, LocalBusFrameKind};
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc};

use super::client::exchange;
use super::deadline::{Deadlines, deadline_ms};
use super::stream::{LAGGED, StreamEnd};
use super::{BusError, BusRequest};

/// The job topic, the only one the daemon-level stream carries.
pub const TOPIC: &str = "account.backup.job.changed";

/// The job requests: the daemon takes them only from an attended
/// connection, so they travel on the job link.
pub const REQUESTS: [&str; 4] = [
    "account.backup.create",
    "account.restore.verify",
    "account.restore.install",
    "account.backup.cancel",
];

/// What the job link reports to its owner.
#[derive(Debug, Clone, PartialEq)]
pub enum JobStreamed {
    /// The subscription is open. `resync`: the cursor was no longer
    /// covered and it follows from the head: read the jobs again.
    Opened {
        /// Subscribed from the head after `resync_required`.
        resync: bool,
        /// The link's close count when the open started (see
        /// [`JobLink::epoch`]): a notice from before a later close request
        /// is about a connection that is going away.
        epoch: u64,
    },
    /// One job row (the body), with the stream cursor.
    Row {
        /// The daemon's job stream cursor.
        seq: Option<u64>,
        /// The whole job row.
        body: Value,
    },
    /// The stream is over (lag, a broken connection, or a refused
    /// subscription); the owner closes the link and decides when to open it
    /// again.
    Ended(StreamEnd),
}

/// The job connection of one scope. Cheap to clone.
#[derive(Clone)]
pub struct JobLink {
    inner: Arc<Inner>,
}

struct Inner {
    socket: PathBuf,
    connect_timeout: Duration,
    deadlines: Deadlines,
    out: mpsc::UnboundedSender<JobStreamed>,
    /// The open connection and the task reading its events.
    link: Mutex<Option<Open>>,
    /// The last stream cursor seen (0: none yet).
    cursor: Arc<AtomicU64>,
    next_id: AtomicU64,
    /// How many closes were asked for. Bumped when a close is asked for,
    /// not when it runs, so an open still in flight is known to be out of
    /// date.
    epoch: AtomicU64,
}

struct Open {
    client: LocalBusClient,
    reader: tokio::task::AbortHandle,
}

impl Drop for Open {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

impl JobLink {
    /// A link to `socket`; nothing connects until [`JobLink::ensure`] or a
    /// request.
    pub fn new(
        socket: PathBuf,
        connect_timeout: Duration,
        deadlines: Deadlines,
        out: mpsc::UnboundedSender<JobStreamed>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                socket,
                connect_timeout,
                deadlines,
                out,
                link: Mutex::new(None),
                cursor: Arc::new(AtomicU64::new(0)),
                next_id: AtomicU64::new(1),
                epoch: AtomicU64::new(0),
            }),
        }
    }

    /// The subscription frame, from `since_seq` when there is one.
    pub fn subscription(since_seq: Option<u64>, deadlines: Deadlines) -> LocalBusFrame {
        let mut body = json!({ "topics": [TOPIC], "attended_frontend": true });
        if let Some(seq) = since_seq {
            body["since_seq"] = json!(seq);
        }
        let mut frame = LocalBusFrame::request("desk_jobs", None, "stream", "bus.subscribe", body);
        frame.deadline_ms = Some(deadline_ms(deadlines.budget("bus.subscribe")));
        frame
    }

    /// Opens the connection and its job subscription, unless open. A
    /// failure is also reported as [`JobStreamed::Ended`].
    pub async fn ensure(&self) -> Result<LocalBusClient, BusError> {
        let mut link = self.inner.link.lock().await;
        // A connection whose reader ended (lag, a broken connection) is
        // dead even before its owner closed it: it is opened again, never
        // handed out.
        if let Some(open) = link.as_ref().filter(|open| !open.reader.is_finished()) {
            return Ok(open.client.clone());
        }
        link.take();
        match self.open().await {
            Ok(open) => {
                let client = open.client.clone();
                *link = Some(open);
                Ok(client)
            }
            Err(failed) => {
                let (end, error) = *failed;
                let _ = self.inner.out.send(JobStreamed::Ended(end));
                Err(error)
            }
        }
    }

    /// Opens the connection and subscribes. A failure says how the stream
    /// ended, with the error (boxed: the pair is large).
    async fn open(&self) -> Result<Open, Box<(StreamEnd, BusError)>> {
        let inner = &self.inner;
        let epoch = inner.epoch.load(Ordering::SeqCst);
        let connected =
            tokio::time::timeout(inner.connect_timeout, LocalBusClient::connect(&inner.socket))
                .await;
        let mut client = match connected {
            Err(_) => {
                return Err(Box::new((StreamEnd::Lost(BusError::Timeout), BusError::Timeout)));
            }
            Ok(Err(error)) => {
                let error = BusError::from(error);
                return Err(Box::new((StreamEnd::Lost(error.clone()), error)));
            }
            Ok(Ok(client)) => client,
        };
        let since = Some(inner.cursor.load(Ordering::Relaxed)).filter(|seq| *seq > 0);
        let mut resync = false;
        let mut answer = subscribe(&client, since, inner.deadlines).await;
        if answer.as_ref().is_err_and(is_resync) {
            // Not covered (pruned, or another daemon): follow from the head
            // and have the owner read the jobs again.
            resync = true;
            inner.cursor.store(0, Ordering::Relaxed);
            answer = subscribe(&client, None, inner.deadlines).await;
        }
        let answer = match answer {
            Ok(answer) => answer,
            Err(error) => {
                let end = match &error {
                    BusError::Daemon { message, .. } => StreamEnd::Unsupported(message.clone()),
                    lost => StreamEnd::Lost(lost.clone()),
                };
                return Err(Box::new((end, error)));
            }
        };
        // The stream's head is the cursor from now on, also when no event
        // arrives on this connection: the next open resumes from it, and a
        // daemon that restarted (its head below it) answers
        // `resync_required`, so the jobs it lost are settled.
        if let Some(head) = answer.get("head_seq").and_then(Value::as_u64) {
            inner.cursor.fetch_max(head, Ordering::Relaxed);
        }
        let _ = inner.out.send(JobStreamed::Opened { resync, epoch });
        let requester = client.clone();
        let (out, cursor) = (inner.out.clone(), inner.cursor.clone());
        let reader = tokio::spawn(async move {
            let end = loop {
                match client.next_event().await {
                    Ok(frame) if frame.kind == LocalBusFrameKind::Event => {
                        if frame.method == LAGGED {
                            let last = frame.body.get("last_delivered_seq").and_then(Value::as_u64);
                            break StreamEnd::Lagged { last_delivered_seq: last };
                        }
                        if frame.method != TOPIC {
                            continue;
                        }
                        if let Some(seq) = frame.seq {
                            cursor.fetch_max(seq, Ordering::Relaxed);
                        }
                        let row = JobStreamed::Row { seq: frame.seq, body: frame.body };
                        if out.send(row).is_err() {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(error) => break StreamEnd::Lost(BusError::from(error)),
                }
            };
            let _ = out.send(JobStreamed::Ended(end));
        });
        Ok(Open { client: requester, reader: reader.abort_handle() })
    }

    /// Sends a job request on the attended connection (opening it first),
    /// and waits for its answer, at most until its deadline.
    pub async fn call(&self, request: &BusRequest) -> Result<Value, BusError> {
        let client = self.ensure().await?;
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let answer =
            exchange(&client, request, self.inner.deadlines, &format!("desk_job_{id}")).await;
        if answer.as_ref().is_err_and(BusError::is_connection_lost) {
            self.close().await;
        }
        answer
    }

    /// Closes the connection (its stream ends); the next request or
    /// [`JobLink::ensure`] opens it again from the last cursor.
    pub async fn close(&self) {
        self.inner.epoch.fetch_add(1, Ordering::SeqCst);
        self.close_now().await;
    }

    /// Asks for the connection to close, at once as far as
    /// [`JobLink::epoch`] goes (an open that started earlier is already
    /// out of date), and closes it in the background.
    pub fn close_soon(&self) {
        self.inner.epoch.fetch_add(1, Ordering::SeqCst);
        let link = self.clone();
        tokio::spawn(async move { link.close_now().await });
    }

    async fn close_now(&self) {
        self.inner.link.lock().await.take();
    }

    /// How many closes were asked for so far. A [`JobStreamed::Opened`]
    /// whose epoch is below this one is about a connection that is being
    /// closed (or is closed).
    pub fn epoch(&self) -> u64 {
        self.inner.epoch.load(Ordering::SeqCst)
    }

    /// The last stream cursor seen, if any.
    pub fn cursor(&self) -> Option<u64> {
        Some(self.inner.cursor.load(Ordering::Relaxed)).filter(|seq| *seq > 0)
    }
}

async fn subscribe(
    client: &LocalBusClient,
    since: Option<u64>,
    deadlines: Deadlines,
) -> Result<Value, BusError> {
    let frame = JobLink::subscription(since, deadlines);
    let budget = deadlines.budget("bus.subscribe");
    tokio::time::timeout(budget, client.request_frame_response(frame))
        .await
        .map_err(|_| BusError::Timeout)?
        .map_err(BusError::from)
        .and_then(|response| match response.kind {
            LocalBusFrameKind::Error => Err(BusError::from_error_body(response.error)),
            _ => Ok(response.body),
        })
}

/// Whether the daemon refused the cursor as no longer covered.
fn is_resync(error: &BusError) -> bool {
    match error {
        BusError::Daemon { code, message, reason_code, .. } => {
            code == "resync_required"
                || reason_code.as_deref() == Some("resync_required")
                || message.starts_with("resync_required")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_subscription_is_daemon_level_attended_and_names_only_the_job_topic() {
        let deadlines = Deadlines::standard();
        let fresh = JobLink::subscription(None, deadlines);
        assert_eq!(fresh.account_id, None);
        assert_eq!(fresh.method, "bus.subscribe");
        assert_eq!(fresh.body["topics"], json!([TOPIC]));
        assert_eq!(fresh.body["attended_frontend"], true);
        assert!(fresh.body.get("since_seq").is_none());
        let resumed = JobLink::subscription(Some(41), deadlines);
        assert_eq!(resumed.body["since_seq"], 41);
    }

    #[test]
    fn a_resync_refusal_is_recognized_in_every_shape() {
        let daemon = |code: &str, message: &str, reason: Option<&str>| BusError::Daemon {
            code: code.to_owned(),
            message: message.to_owned(),
            reason_code: reason.map(str::to_owned),
            details: None,
            cause: None,
        };
        assert!(is_resync(&daemon("resync_required", "resync_required", None)));
        assert!(is_resync(&daemon("BackupRefused", "x", Some("resync_required"))));
        assert!(!is_resync(&daemon("BackupRefused", "daemon_scope_jobs_only", None)));
        assert!(!is_resync(&BusError::Timeout));
    }
}
