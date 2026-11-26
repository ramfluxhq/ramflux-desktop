// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Replies of the card runtime (`card.session.*`), typed.
//!
//! Every reply carries `result`, `render_allowed` and `fallback_text`. When
//! `render_allowed` is false the card shows its fallback text only. A
//! refusal or failure is classified by [`failure`]: an absent runtime (a
//! daemon built without `card-runtime`) is the only reason the desktop
//! raises `backend.gap.surfaces`.

use ramflux_desktop_core::CardBatch;
use serde_json::Value;
use thiserror::Error;

use crate::bus::BusError;
use crate::plan::card_session::SlotOutcome;

/// The message of a daemon built without the card runtime.
const RUNTIME_UNAVAILABLE: &str = "card runtime unavailable";
/// Prefix of every typed card refusal reason.
const REASON_PREFIX: &str = "card_session_";

/// A reply that does not follow the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("card reply is malformed")]
pub struct ReplyError;

/// The `result` of a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardResult {
    /// A session was opened; or a link slot was confirmed.
    Opened,
    /// A link slot was cancelled.
    Cancelled,
    /// A link slot could not be answered.
    SlotUnavailable,
    /// The signed update was applied.
    Updated,
    /// A locale or theme change was applied.
    Presented,
    /// A host event was applied.
    Edited,
    /// A link activation needs the host's sheet (`host_request`).
    OpenUrl,
    /// The Button was consumed; call submit next.
    PendingSubmit,
    /// The submit reached the daemon.
    Submitted,
    /// The session was suspended.
    Suspended,
    /// The session was resumed under a fresh identity.
    Resumed,
    /// The session was disposed.
    Disposed,
    /// The host's approval verdict was recorded (not yet an author effect).
    ApprovalDecision,
    /// The request was refused (`reason`).
    Refused,
    /// The request was for a state that moved on; the caller reverts.
    StaleDropped,
    /// A newer publication replaced the session.
    Superseded,
}

impl CardResult {
    fn parse(text: &str) -> Result<Self, ReplyError> {
        Ok(match text {
            "opened" => Self::Opened,
            "cancelled" => Self::Cancelled,
            "unavailable" => Self::SlotUnavailable,
            "updated" => Self::Updated,
            "presented" => Self::Presented,
            "edited" => Self::Edited,
            "open_url" => Self::OpenUrl,
            "pending_submit" => Self::PendingSubmit,
            "submitted" => Self::Submitted,
            "suspended" => Self::Suspended,
            "resumed" => Self::Resumed,
            "disposed" => Self::Disposed,
            "approval_decision" => Self::ApprovalDecision,
            "refused" => Self::Refused,
            "stale_dropped" => Self::StaleDropped,
            "superseded" => Self::Superseded,
            _ => return Err(ReplyError),
        })
    }
}

/// A host request the runtime raised with a reply.
#[derive(Debug, Clone, PartialEq)]
pub enum HostRequest {
    /// Show the link sheet for `url` (canonical https, validated by the
    /// runtime). Open only after the slot answers `opened`.
    OpenUrl {
        /// Slot id to answer.
        slot_id: String,
        /// Request id to answer (secret; never logged).
        request_id: String,
        /// The exact URL to show.
        url: String,
        /// Token for the slot answer.
        token: Value,
    },
    /// The host's verdict on an approval slot. Verified intent only.
    ApprovalDecision {
        /// Confirm or deny.
        outcome: SlotOutcome,
        /// The record the verdict is bound to.
        request_ref: String,
        /// The task binding, when the approval has one.
        task_binding: Option<Value>,
    },
}

/// The receipt of a submit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitReceipt {
    /// Message id the daemon assigned.
    pub message_id: String,
    /// Surface version that was submitted.
    pub surface_version: u64,
    /// Surface hash that was submitted.
    pub surface_hash: String,
}

/// The author's verdict on an action, when the runtime reports one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionOutcome {
    /// `accepted`, `refused`, `superseded`, ...
    pub status: String,
    /// The typed reason, when there is one.
    pub reason: Option<String>,
    /// The card that replaced this one, when verified.
    pub replacement_ref: Option<String>,
}

/// One typed reply.
#[derive(Debug, Clone, PartialEq)]
pub struct CardReply {
    /// What happened.
    pub result: CardResult,
    /// The session the reply is about, when it names one.
    pub session: Option<u64>,
    /// Whether the runtime allows drawing the card (false: fallback only).
    pub render_allowed: bool,
    /// The author's fallback text.
    pub fallback_text: String,
    /// The typed reason of a refusal.
    pub reason: Option<String>,
    /// The batch to apply to the held tree.
    pub batch: Option<CardBatch>,
    /// The host request, when the reply raises one.
    pub host_request: Option<HostRequest>,
    /// The submit receipt, after a submit.
    pub submit: Option<SubmitReceipt>,
    /// The author's draft was discarded in favour of the signed update.
    pub draft_conflict: bool,
    /// The action verdict, when reported.
    pub action_outcome: Option<ActionOutcome>,
}

/// Parses the `result` object of a card reply.
pub fn reply(value: &Value) -> Result<CardReply, ReplyError> {
    let result = CardResult::parse(value["result"].as_str().ok_or(ReplyError)?)?;
    let render_allowed = value["render_allowed"].as_bool().ok_or(ReplyError)?;
    let fallback_text = value["fallback_text"].as_str().ok_or(ReplyError)?.to_owned();
    let batch = match value.get("batch") {
        Some(batch) if render_allowed => Some(decode(batch)?),
        _ => None,
    };
    let host_request = match value.get("host_request") {
        Some(request) if render_allowed => Some(host_request(request)?),
        _ => None,
    };
    let submit = match value.get("submit") {
        Some(submit) if render_allowed => Some(SubmitReceipt {
            message_id: string(submit, "message_id")?,
            surface_version: submit["surface_version"].as_u64().ok_or(ReplyError)?,
            surface_hash: string(submit, "surface_hash")?,
        }),
        _ => None,
    };
    let action_outcome = value.get("action_outcome").map(action).transpose()?;
    Ok(CardReply {
        result,
        session: value["session"].as_u64(),
        render_allowed,
        fallback_text,
        reason: value["reason"].as_str().map(str::to_owned),
        batch,
        host_request,
        submit,
        draft_conflict: value["draft_conflict"].as_bool().unwrap_or(false),
        action_outcome,
    })
}

fn decode(batch: &Value) -> Result<CardBatch, ReplyError> {
    serde_json::from_value(batch.clone()).map_err(|_| ReplyError)
}

fn string(value: &Value, field: &str) -> Result<String, ReplyError> {
    value[field].as_str().map(str::to_owned).ok_or(ReplyError)
}

fn host_request(request: &Value) -> Result<HostRequest, ReplyError> {
    match request["kind"].as_str().ok_or(ReplyError)? {
        "open_url" => Ok(HostRequest::OpenUrl {
            slot_id: string(request, "slot_id")?,
            request_id: string(request, "request_id")?,
            url: string(request, "url")?,
            token: request.get("token").cloned().ok_or(ReplyError)?,
        }),
        "approval_decision" => Ok(HostRequest::ApprovalDecision {
            outcome: slot_outcome(request["outcome"].as_str().ok_or(ReplyError)?)?,
            request_ref: string(request, "request_ref")?,
            task_binding: request.get("task_binding").filter(|v| !v.is_null()).cloned(),
        }),
        _ => Err(ReplyError),
    }
}

fn slot_outcome(text: &str) -> Result<SlotOutcome, ReplyError> {
    Ok(match text {
        "confirm" => SlotOutcome::Confirm,
        "deny" => SlotOutcome::Deny,
        "cancel" => SlotOutcome::Cancel,
        "unavailable" => SlotOutcome::Unavailable,
        _ => return Err(ReplyError),
    })
}

fn action(outcome: &Value) -> Result<ActionOutcome, ReplyError> {
    Ok(ActionOutcome {
        status: string(outcome, "status")?,
        reason: outcome["reason"].as_str().map(str::to_owned),
        replacement_ref: outcome["replacement_ref"].as_str().map(str::to_owned),
    })
}

/// Why a card call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardFailure {
    /// The daemon has no card runtime (or no `card.session.*` methods).
    /// The only case that raises `backend.gap.surfaces`.
    RuntimeUnavailable,
    /// A typed refusal (`card_session_*`), by its reason key.
    Refused(String),
    /// A timeout or a broken connection: the call may be retried later.
    Other,
}

/// Classifies a failed card call.
#[must_use]
pub fn failure(error: &BusError) -> CardFailure {
    if error.is_unknown_method() {
        return CardFailure::RuntimeUnavailable;
    }
    let BusError::Daemon { message, .. } = error else {
        return CardFailure::Other;
    };
    if message.contains(RUNTIME_UNAVAILABLE) {
        return CardFailure::RuntimeUnavailable;
    }
    match error.reason_code().or(Some(message.as_str())) {
        Some(reason) if reason.starts_with(REASON_PREFIX) => {
            CardFailure::Refused(reason.to_owned())
        }
        _ => CardFailure::Other,
    }
}
