// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Replies of the card runtime (`card.session.*`), typed.
//!
//! Every reply carries `result`, `render_allowed` and `fallback_text`. When
//! `render_allowed` is false the card shows its fallback text only. A
//! refusal or failure is classified by [`failure`]: an absent runtime (a
//! daemon built without `card-runtime`) is the only reason the desktop
//! raises `backend.gap.surfaces`.

use ramflux_desktop_core::{
    CardBatch, CardEvent, CardLink, CardReceipt, CardSlotOutcome as SlotOutcome, DomainEvent,
    SurfaceId,
};
use serde_json::Value;
use thiserror::Error;

use crate::bus::BusError;

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

/// The domain events of one card call's outcome: its reply, or its failure.
/// A daemon without the runtime raises the card gap (every card call is then
/// unsupported); a typed refusal is the card's own refusal; anything else is
/// a refused call the card shows in place, never retried here.
pub fn outcome(method: &str, body: &Value, result: &Result<Value, BusError>) -> Vec<DomainEvent> {
    match result {
        Ok(answer) => match reply(answer) {
            Ok(reply) => events(method, body, reply),
            Err(ReplyError) => vec![refused_event(body, "card_session_reply")],
        },
        Err(error) => match failure(error) {
            CardFailure::RuntimeUnavailable => vec![DomainEvent::CardRuntimeUnavailable],
            CardFailure::Refused(reason) => vec![refused_event(body, &reason)],
            CardFailure::Other => vec![refused_event(body, "card_session_failed")],
        },
    }
}

fn refused_event(body: &Value, reason: &str) -> DomainEvent {
    DomainEvent::Card(CardEvent::Refused {
        surface_id: SurfaceId::new(body["surface_id"].as_str().unwrap_or_default()),
        reason: reason.to_owned(),
    })
}

/// The domain events one card call's reply means. `body` is the call's body
/// (it names the surface, and the URL of a link answer); `method` is the bus
/// method the call was made with.
///
/// A refusal carries its reason key. A reply the runtime does not allow to
/// draw (`render_allowed` false) makes the card fall back to its text.
pub fn events(method: &str, body: &Value, reply: CardReply) -> Vec<DomainEvent> {
    let surface_id = SurfaceId::new(body["surface_id"].as_str().unwrap_or_default());
    let event = |event: CardEvent| DomainEvent::Card(event);
    let refused = |reason: &str| {
        vec![event(CardEvent::Refused {
            surface_id: surface_id.clone(),
            reason: reason.to_owned(),
        })]
    };
    if reply.result == CardResult::Refused {
        return refused(reply.reason.as_deref().unwrap_or("card_session_refused"));
    }
    if reply.result == CardResult::StaleDropped {
        return refused("stale_dropped");
    }
    if reply.result == CardResult::Superseded {
        return vec![event(CardEvent::Stale { surface_id })];
    }
    let batch = |b: CardBatch| Box::new(b);
    let mut out = Vec::new();
    match (method, reply.result) {
        (CARD_OPEN, CardResult::Opened) => match (reply.session, reply.batch) {
            (Some(session), Some(b)) => out.push(event(CardEvent::Opened {
                surface_id: surface_id.clone(),
                session,
                batch: batch(b),
            })),
            _ => out.push(event(CardEvent::Fallback { surface_id: surface_id.clone() })),
        },
        (CARD_RESUME, CardResult::Resumed) => match (reply.session, reply.batch) {
            (Some(session), Some(b)) => out.push(event(CardEvent::Resumed {
                surface_id: surface_id.clone(),
                session,
                batch: batch(b),
            })),
            _ => out.push(event(CardEvent::Stale { surface_id: surface_id.clone() })),
        },
        (CARD_UPDATE | CARD_PRESENT | CARD_EVENT | CARD_SLOT, _) if !reply.render_allowed => {
            out.push(event(CardEvent::Fallback { surface_id: surface_id.clone() }));
        }
        (CARD_UPDATE, CardResult::Updated) | (CARD_PRESENT, CardResult::Presented) => {
            if let Some(b) = reply.batch {
                out.push(event(CardEvent::Batch {
                    surface_id: surface_id.clone(),
                    batch: batch(b),
                    draft_conflict: reply.draft_conflict,
                }));
            }
        }
        (CARD_EVENT, CardResult::Edited | CardResult::PendingSubmit | CardResult::OpenUrl) => {
            if let Some(b) = reply.batch {
                out.push(event(CardEvent::Batch {
                    surface_id: surface_id.clone(),
                    batch: batch(b),
                    draft_conflict: reply.draft_conflict,
                }));
            }
            if reply.result == CardResult::PendingSubmit {
                out.push(event(CardEvent::PendingSubmit { surface_id: surface_id.clone() }));
            }
            if let Some(HostRequest::OpenUrl { slot_id, request_id, url, token }) =
                reply.host_request
            {
                out.push(event(CardEvent::OpenUrl {
                    surface_id: surface_id.clone(),
                    link: CardLink { slot_id, request_id, url, token },
                }));
            }
        }
        (CARD_SLOT, CardResult::Opened) => {
            // The browser opens for the URL the slot answer was given for.
            if let Some(url) = body["url"].as_str() {
                out.push(event(CardEvent::LinkOpened {
                    surface_id: surface_id.clone(),
                    url: url.to_owned(),
                }));
            }
            out.push(event(CardEvent::LinkClosed { surface_id: surface_id.clone() }));
        }
        (CARD_SLOT, CardResult::Cancelled | CardResult::SlotUnavailable) => {
            out.push(event(CardEvent::LinkClosed { surface_id: surface_id.clone() }));
        }
        (CARD_SLOT, CardResult::ApprovalDecision) => {
            if let Some(HostRequest::ApprovalDecision { outcome, .. }) = reply.host_request {
                out.push(event(CardEvent::ApprovalVerdict {
                    surface_id: surface_id.clone(),
                    outcome,
                }));
            }
        }
        (CARD_SUBMIT, CardResult::Submitted) => {
            if let Some(receipt) = reply.submit {
                out.push(event(CardEvent::Submitted {
                    surface_id: surface_id.clone(),
                    receipt: CardReceipt {
                        message_id: receipt.message_id,
                        surface_version: receipt.surface_version,
                        surface_hash: receipt.surface_hash,
                    },
                }));
            }
        }
        (CARD_SUSPEND, CardResult::Suspended) => {
            out.push(event(CardEvent::Suspended { surface_id: surface_id.clone() }));
        }
        (CARD_DISPOSE, CardResult::Disposed) => {
            out.push(event(CardEvent::Disposed { surface_id: surface_id.clone() }));
        }
        _ => return refused("card_session_reply"),
    }
    out
}

const CARD_OPEN: &str = "card.session.open";
const CARD_UPDATE: &str = "card.session.update";
const CARD_PRESENT: &str = "card.session.present";
const CARD_EVENT: &str = "card.session.event";
const CARD_SLOT: &str = "card.session.slot";
const CARD_SUBMIT: &str = "card.session.submit";
const CARD_SUSPEND: &str = "card.session.suspend";
const CARD_RESUME: &str = "card.session.resume";
const CARD_DISPOSE: &str = "card.session.dispose";
