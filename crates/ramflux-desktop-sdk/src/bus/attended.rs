// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Requests the daemon takes only from the attended session of an account,
//! on a connection of their own (DESK-AGENT-01).
//!
//! Exporting the credentials of an AI participant is allowed to the
//! custodian's own unlocked session only: the daemon checks that the
//! connection the request arrives on opened an attended subscription for
//! the account named in the frame. The account scopes' request connections
//! are shared with every other request of the scope and never claim to be
//! attended, so each such request runs on a short-lived connection that
//! first opens that subscription (`topics: []`, `attended_frontend: true`,
//! as the reference CLI does), sends the one request, and is closed when
//! the answer is in. Nothing here keeps a connection, a path's content or a
//! credential: the answer only says where the bundle was written.

use std::path::Path;
use std::time::Duration;

use ramflux_sdk::LocalBusClient;
use serde_json::{Value, json};

use super::client::exchange;
use super::deadline::Deadlines;
use super::{BusError, BusRequest};

/// The requests that travel on an attended connection of their own.
pub const REQUESTS: [&str; 1] = ["account.export_agent_credentials"];

/// Whether `method` must be sent with [`call`].
pub fn is_attended(method: &str) -> bool {
    REQUESTS.contains(&method)
}

/// The request that makes a connection the attended session of `account`.
pub fn attend(account: &str) -> BusRequest {
    BusRequest::new(
        Some(account),
        "stream",
        "subscription.open",
        json!({ "topics": [], "attended_frontend": true }),
    )
}

/// Opens a connection, attends `request.account` on it, sends `request` and
/// returns the daemon's answer.
///
/// # Errors
///
/// [`BusError`] when the connection cannot be made, the daemon refuses to
/// attend the account, or the request fails.
pub async fn call(
    socket: &Path,
    connect_timeout: Duration,
    deadlines: Deadlines,
    request: &BusRequest,
) -> Result<Value, BusError> {
    let Some(account) = request.account.as_deref() else {
        return Err(BusError::daemon("ValidationFailed", "an attended request names its account"));
    };
    let client = tokio::time::timeout(connect_timeout, LocalBusClient::connect(socket))
        .await
        .map_err(|_| BusError::Transport("the daemon socket did not accept in time".to_owned()))?
        .map_err(BusError::from)?;
    exchange(&client, &attend(account), deadlines, "desk_attended").await?;
    exchange(&client, request, deadlines, "desk_attended_request").await
}
