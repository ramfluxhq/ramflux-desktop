// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The push subscription of one unlocked account on a daemon without the
//! event stream (`subscription.open`), used only while the source falls back
//! to polling.
//!
//! Such a daemon pushes a few topics (approval requests and decisions, tool
//! starts, A2UI action records); everything else is polled. The connection
//! is dedicated to the account's events, apart from its request connection,
//! so a burst of events never queues behind requests (or the other way
//! round). A daemon with the stream is followed through [`super::stream`].

use std::path::Path;
use std::time::Duration;

use ramflux_sdk::{LocalBusClient, LocalBusFrame, LocalBusFrameKind};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::BusError;
use super::deadline::{Deadlines, deadline_ms};

/// Topics a daemon without the event stream pushes.
pub const PUSHED_TOPICS: [&str; 6] = [
    "mcp.approval.request",
    "mcp.approval.granted",
    "mcp.approval.denied",
    "mcp.tool.started",
    "ramflux.a2ui.action_submitted",
    "ramflux.a2ui.action_result",
];

/// Every topic the push subscription asks for: the pushed ones, voice
/// call rows and contact requests (both read directly).
pub fn topics() -> Vec<&'static str> {
    let mut topics = PUSHED_TOPICS.to_vec();
    topics.push(crate::translate::stream::CALL_TOPIC);
    topics.extend(crate::translate::contact_requests::TOPICS);
    topics
}

/// What the subscription reports.
#[derive(Debug, Clone, PartialEq)]
pub enum Pushed {
    /// The subscription is registered.
    Opened,
    /// One event frame.
    Event {
        /// Topic (the frame's method).
        topic: String,
        /// Event body.
        body: Value,
    },
    /// The connection ended; the owner resubscribes after a re-snapshot.
    Lost(BusError),
}

/// Opens the subscription for `account` and forwards its events until the
/// connection ends or the receiver is dropped.
pub async fn run(
    socket: &Path,
    account: String,
    connect_timeout: Duration,
    deadlines: Deadlines,
    out: mpsc::UnboundedSender<Pushed>,
) {
    let ended = follow(socket, account, connect_timeout, deadlines, &out).await;
    let _ = out.send(Pushed::Lost(ended));
}

async fn follow(
    socket: &Path,
    account: String,
    connect_timeout: Duration,
    deadlines: Deadlines,
    out: &mpsc::UnboundedSender<Pushed>,
) -> BusError {
    let connected = tokio::time::timeout(connect_timeout, LocalBusClient::connect(socket)).await;
    let mut client = match connected {
        Err(_) => return BusError::Timeout,
        Ok(Err(error)) => return BusError::from(error),
        Ok(Ok(client)) => client,
    };
    let mut open = LocalBusFrame::request(
        "desk_subscription",
        Some(account),
        "subscription",
        "subscription.open",
        json!({ "topics": topics(), "attended_frontend": true }),
    );
    open.deadline_ms = Some(deadline_ms(deadlines.budget("subscription.open")));
    if let Err(error) = client.request_frame(open).await {
        return BusError::from(error);
    }
    if out.send(Pushed::Opened).is_err() {
        return BusError::Transport("subscriber gone".to_owned());
    }
    loop {
        match client.next_event().await {
            Ok(frame) if frame.kind == LocalBusFrameKind::Event => {
                let event = Pushed::Event { topic: frame.method, body: frame.body };
                if out.send(event).is_err() {
                    return BusError::Transport("subscriber gone".to_owned());
                }
            }
            Ok(_) => {}
            Err(error) => return BusError::from(error),
        }
    }
}
