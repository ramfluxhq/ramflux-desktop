// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Running a username claim (see `plan::username`): the daemon prepares the
//! draft, this process authorizes it with the recovery phrase (in a blocking
//! task, the phrase dropped with the call), the daemon submits the signed
//! records, and the status is polled until the directory commits or aborts
//! them, or the wait runs out (the claim then stays "publishing").
//!
//! The phrase is never sent to the daemon and never logged; the requests
//! carry only the public draft and the signed records.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ramflux_sdk::SdkError;
use ramflux_sdk::username_directory::{
    UsernameChange, UsernameDraft, authorize_username_draft, normalize_name,
};
use serde_json::{Value, json};

use crate::bus::client::BusClient;
use crate::bus::{BusError, BusRequest, idempotency};
use crate::plan::username::ClaimJob;
use crate::username_center;

/// How often the status is read while the directory works on the claim.
const POLL_EVERY: Duration = Duration::from_secs(2);
/// How long the status is read before the claim is left "publishing".
const POLL_FOR: Duration = Duration::from_secs(30);

/// Runs `job`; the answer is the last status the daemon gave (or the
/// daemon's prepare answer when the change is a no-op).
pub(super) async fn claim(client: &BusClient, job: ClaimJob) -> Result<Value, BusError> {
    let ClaimJob { account, name, phrase } = job;
    let acct = account.as_str();
    let pin = username_center::current().ok_or_else(|| {
        BusError::daemon("directory_unavailable", "no username centre is configured")
    })?;
    let name = normalize_name(&name).map_err(refusal)?;
    let change = UsernameChange::Set { name: name.clone() };
    let prepared = client
        .call(&BusRequest::new(
            Some(acct),
            "username",
            "username.mutate",
            json!({ "phase": "prepare", "change": { "change": "set", "name": name } }),
        ))
        .await?;
    let Some(draft) = prepared.get("draft").filter(|draft| !draft.is_null()) else {
        // Nothing to publish: the status the daemon gave says what is in force.
        return Ok(prepared);
    };
    let draft: UsernameDraft = serde_json::from_value(draft.clone())
        .map_err(|_| BusError::daemon("invalid_response", "the username draft did not parse"))?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| BusError::daemon("clock_unavailable", "the system clock is unavailable"))?
        .as_secs();
    let words = phrase.into_zeroizing();
    let signed = tokio::task::spawn_blocking(move || {
        authorize_username_draft(words, draft, &pin, &change, now)
    })
    .await
    .map_err(|_| BusError::daemon("internal", "the authorization task stopped"))?
    .map_err(refusal)?;
    let operation_id = signed.operation.body.operation_id.clone();
    let signed = serde_json::to_value(&signed)
        .map_err(|_| BusError::daemon("internal", "the signed change did not encode"))?;
    let key = idempotency::fresh_key().map_err(|message| BusError::daemon("internal", message))?;
    let submit = BusRequest::new(
        Some(acct),
        "username",
        "username.mutate",
        json!({ "phase": "submit", "signed": signed }),
    )
    .with_operation_id(key);
    let mut status = client.call(&submit).await?;
    let deadline = Instant::now() + POLL_FOR;
    while status.get("state").and_then(Value::as_str) == Some("pending")
        && Instant::now() < deadline
    {
        tokio::time::sleep(POLL_EVERY).await;
        status = client
            .call(&BusRequest::new(
                Some(acct),
                "username",
                "username.status",
                json!({ "operation_id": operation_id }),
            ))
            .await?;
    }
    Ok(status)
}

/// A refusal of the SDK's own username checks, as the daemon would give it:
/// the directory code (`username_taken`, `authentication_required`, ...).
fn refusal(error: SdkError) -> BusError {
    match error {
        SdkError::UsernameDirectoryRefused(code) => BusError::daemon(code.as_str(), code.as_str()),
        _ => BusError::daemon("internal", "the username could not be authorized"),
    }
}
