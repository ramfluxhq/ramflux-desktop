// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The event stream of one unlocked account (`bus.subscribe`).
//!
//! The daemon keeps a durable journal of what changed per account, each
//! entry with a sequence number. Subscribing with `since_seq` replays every
//! entry after it, then follows live, in order. A cursor older than what the
//! daemon still keeps is refused with `resync_required`: the owner then
//! subscribes from the head and reloads its state through the usual reads.
//! When the subscriber falls behind, the daemon (or the SDK client) sends a
//! final `stream.lagged` and stops: the owner resubscribes from the last
//! event it applied and loses nothing.
//!
//! Event bodies carry ids and metadata, never message content. The
//! connection is dedicated to the stream, apart from the account's request
//! connection.

use std::path::Path;
use std::time::Duration;

use ramflux_sdk::{LocalBusClient, LocalBusFrame, LocalBusFrameKind, SdkError};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::BusError;
use super::deadline::{Deadlines, deadline_ms};

/// The topic of the daemon's last frame before it stops a stream that fell
/// behind.
pub const LAGGED: &str = "stream.lagged";

/// What the stream reports to its owner.
#[derive(Debug, Clone, PartialEq)]
pub enum Streamed {
    /// The daemon accepted the subscription.
    Opened {
        /// The newest journal entry when it accepted.
        head_seq: u64,
        /// The oldest journal entry it still keeps.
        floor_seq: u64,
    },
    /// One journal entry.
    Event {
        /// Its sequence number (`None` from a daemon without a journal).
        seq: Option<u64>,
        /// Topic (the frame's method).
        topic: String,
        /// Metadata body (ids, states; never content).
        body: Value,
    },
    /// The stream is over; nothing more arrives on it.
    Ended(StreamEnd),
}

/// Why a stream ended.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEnd {
    /// The daemon no longer keeps the entries after the cursor: reload,
    /// then follow from the head.
    ResyncRequired,
    /// The subscriber fell behind; resume from the last applied entry.
    Lagged {
        /// The last entry delivered before the daemon stopped, if it said.
        last_delivered_seq: Option<u64>,
    },
    /// The daemon does not offer the stream (an older daemon, or it
    /// refused the subscription): its words.
    Unsupported(String),
    /// The account is not open (locked).
    NotOpen,
    /// The connection could not be made or broke.
    Lost(BusError),
}

/// What to subscribe to.
#[derive(Debug, Clone)]
pub struct Subscription {
    /// The account.
    pub account: String,
    /// Topics (exact names).
    pub topics: Vec<&'static str>,
    /// Replay the entries after this one; `None` follows from the head.
    pub since_seq: Option<u64>,
}

/// Subscribes and forwards what arrives until the stream ends or the
/// receiver is dropped. Always ends with [`Streamed::Ended`] (unless the
/// receiver is gone).
pub async fn run(
    socket: &Path,
    subscription: Subscription,
    connect_timeout: Duration,
    deadlines: Deadlines,
    out: mpsc::UnboundedSender<Streamed>,
) {
    let end = follow(socket, subscription, connect_timeout, deadlines, &out).await;
    let _ = out.send(Streamed::Ended(end));
}

/// The subscription request frame.
pub fn request(subscription: &Subscription, deadlines: Deadlines) -> LocalBusFrame {
    let mut body = json!({ "topics": subscription.topics, "attended_frontend": true });
    if let Some(seq) = subscription.since_seq {
        body["since_seq"] = json!(seq);
    }
    let mut frame = LocalBusFrame::request(
        "desk_stream",
        Some(subscription.account.clone()),
        "bus",
        "bus.subscribe",
        body,
    );
    frame.deadline_ms = Some(deadline_ms(deadlines.budget("bus.subscribe")));
    frame
}

async fn follow(
    socket: &Path,
    subscription: Subscription,
    connect_timeout: Duration,
    deadlines: Deadlines,
    out: &mpsc::UnboundedSender<Streamed>,
) -> StreamEnd {
    let connected = tokio::time::timeout(connect_timeout, LocalBusClient::connect(socket)).await;
    let mut client = match connected {
        Err(_) => return StreamEnd::Lost(BusError::Timeout),
        Ok(Err(error)) => return StreamEnd::Lost(BusError::from(error)),
        Ok(Ok(client)) => client,
    };
    let requester = client.clone();
    let frame = request(&subscription, deadlines);
    let answer = requester.request_frame(frame);
    tokio::pin!(answer);
    let mut opened = false;
    let gone = || StreamEnd::Lost(BusError::Transport("subscriber gone".to_owned()));
    loop {
        // The daemon queues its answer before the first replayed entry and
        // the client hands answers over before events, so the answer is
        // read first when both are ready.
        tokio::select! {
            biased;
            answer = &mut answer, if !opened => match answer {
                Ok(body) => {
                    opened = true;
                    let seq = |field: &str| body.get(field).and_then(Value::as_u64).unwrap_or(0);
                    let event =
                        Streamed::Opened { head_seq: seq("head_seq"), floor_seq: seq("floor_seq") };
                    if out.send(event).is_err() {
                        return gone();
                    }
                }
                Err(error) => return refused(error),
            },
            event = client.next_event() => match event {
                Ok(frame) if frame.kind == LocalBusFrameKind::Event => {
                    if frame.method == LAGGED {
                        let last = frame.body.get("last_delivered_seq").and_then(Value::as_u64);
                        return StreamEnd::Lagged { last_delivered_seq: last };
                    }
                    let event =
                        Streamed::Event { seq: frame.seq, topic: frame.method, body: frame.body };
                    if out.send(event).is_err() {
                        return gone();
                    }
                }
                Ok(_) => {}
                Err(error) => return StreamEnd::Lost(BusError::from(error)),
            },
        }
    }
}

/// Classifies a refused subscription.
pub fn refused(error: SdkError) -> StreamEnd {
    if let SdkError::LocalBus(text) = &error
        && text.starts_with("resync_required")
    {
        return StreamEnd::ResyncRequired;
    }
    match BusError::from(error) {
        BusError::Daemon { message, .. } if message.contains("account not open") => {
            StreamEnd::NotOpen
        }
        BusError::Daemon { message, .. } => StreamEnd::Unsupported(message),
        lost => StreamEnd::Lost(lost),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(text: &str) -> SdkError {
        SdkError::LocalBus(text.to_owned())
    }

    #[test]
    fn refusals_are_told_apart() {
        assert_eq!(
            refused(local("resync_required: local bus error: resync_required")),
            StreamEnd::ResyncRequired
        );
        assert_eq!(
            refused(local("ValidationFailed: local bus error: account not open: a")),
            StreamEnd::NotOpen
        );
        assert_eq!(
            refused(local("ValidationFailed: local bus error: unsupported method bus.subscribe")),
            StreamEnd::Unsupported("local bus error: unsupported method bus.subscribe".to_owned())
        );
        assert!(matches!(
            refused(local("local bus connection closed")),
            StreamEnd::Lost(BusError::Transport(_))
        ));
        assert_eq!(refused(local("request deadline exceeded")), StreamEnd::Lost(BusError::Timeout));
    }

    #[test]
    fn the_request_names_the_cursor_only_when_there_is_one() {
        let mut subscription = Subscription {
            account: "a".to_owned(),
            topics: vec!["message.received"],
            since_seq: None,
        };
        let deadlines = Deadlines::uniform(Duration::from_secs(1));
        let fresh = request(&subscription, deadlines);
        assert_eq!(fresh.method, "bus.subscribe");
        assert_eq!(fresh.account_id.as_deref(), Some("a"));
        assert!(fresh.body.get("since_seq").is_none());
        assert_eq!(fresh.body["attended_frontend"], true);
        subscription.since_seq = Some(41);
        assert_eq!(request(&subscription, deadlines).body["since_seq"], 41);
    }
}
