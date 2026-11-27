// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Replies of the card runtime (`card.session.*`), typed.
//!
//! Every reply carries `result`, `render_allowed` and `fallback_text`. When
//! `render_allowed` is false the card shows its fallback text only. Every
//! `event` and `slot` reply that may be drawn carries a `batch` (an empty
//! one says nothing changed). A refusal or failure is classified by
//! [`failure`] from the typed `reason_code` (or the error `code`), never
//! from the message.

use ramflux_desktop_core::{
    CARD_STALE_REASON, CardApproval, CardBatch, CardEvent, CardImage, CardLimit, CardLink,
    CardReceipt, CardSlotOutcome as SlotOutcome, DomainEvent, SurfaceId,
};
use serde_json::Value;
use thiserror::Error;

use crate::bus::BusError;

/// Prefix of every typed card refusal reason.
const REASON_PREFIX: &str = "card_session_";
/// The resident-card cap of the daemon process.
const LIMIT_PROCESS: &str = "card_limit_exceeded";
/// The sessions-per-account cap.
const LIMIT_ACCOUNT: &str = "card_session_limit";
/// A session the runtime dropped.
const EXPIRED: [&str; 2] = ["card_session_expired", "card_session_inactive"];

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
    /// A host event was applied and a field's validation answered.
    Validation,
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
            "validation" => Self::Validation,
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
        /// The complete `task.resolve` body (confirm and deny only), to be
        /// sent unchanged.
        task_resolve: Option<Value>,
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
            task_resolve: request.get("task_resolve").filter(|v| !v.is_null()).cloned(),
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
    /// The session is gone (`card_session_expired`, `card_session_inactive`).
    Expired,
    /// A limit of the runtime (`card_limit_exceeded`, `card_session_limit`).
    Limit(CardLimit),
    /// A typed refusal (`card_session_*`), by its reason key.
    Refused(String),
    /// A timeout, a broken connection or an untyped refusal: the call may
    /// be retried later.
    Other,
}

/// Classifies a failed card call from its typed reason. The resume cap is
/// untyped at the pinned core (an invalid-argument refusal of a resume), so
/// that one answer is a limit too.
#[must_use]
pub fn failure(method: &str, error: &BusError) -> CardFailure {
    let BusError::Daemon { code, reason_code, .. } = error else {
        return CardFailure::Other;
    };
    let token = reason_code.as_deref().unwrap_or(code.as_str());
    if EXPIRED.contains(&token) {
        CardFailure::Expired
    } else if token == LIMIT_PROCESS {
        CardFailure::Limit(CardLimit::Process)
    } else if token == LIMIT_ACCOUNT {
        CardFailure::Limit(CardLimit::Account)
    } else if token.starts_with(REASON_PREFIX) {
        CardFailure::Refused(token.to_owned())
    } else if method == CARD_RESUME
        && reason_code.is_none()
        && matches!(code.as_str(), "InvalidArgument" | "ValidationFailed")
    {
        CardFailure::Limit(CardLimit::Process)
    } else {
        CardFailure::Other
    }
}

/// The domain events of one card call's outcome: its reply, or its failure.
/// A typed refusal is the card's own refusal; a dropped session is the
/// card's to open again; a limit is retried once by the app; anything else
/// is a refused call the card shows in place, never retried here. A submit
/// whose answer is unknown (a timeout or a broken connection, or a reply
/// that cannot be read) is no refusal: the button stays consumed and the
/// user may retry it. A host event call always ends with
/// [`CardEvent::EventSettled`], whatever happened.
pub fn outcome(method: &str, body: &Value, result: &Result<Value, BusError>) -> Vec<DomainEvent> {
    let unknown_submit = || {
        vec![DomainEvent::Card(CardEvent::SubmitFailed {
            surface_id: SurfaceId::new(body["surface_id"].as_str().unwrap_or_default()),
        })]
    };
    let surface_id = || SurfaceId::new(body["surface_id"].as_str().unwrap_or_default());
    if method == CARD_APPROVAL_FINISH {
        return approval_outcome(body, result);
    }
    let mut out = match result {
        Ok(answer) => match reply(answer) {
            Ok(reply) => events(method, body, reply),
            Err(ReplyError) if method == CARD_SUBMIT => unknown_submit(),
            Err(ReplyError) => vec![refused_event(body, "card_session_reply")],
        },
        Err(error) => match failure(method, error) {
            // The release asked for is done: the runtime dropped it already.
            CardFailure::Expired if method == CARD_DISPOSE => {
                vec![DomainEvent::Card(CardEvent::Disposed { surface_id: surface_id() })]
            }
            CardFailure::Expired => {
                vec![DomainEvent::Card(CardEvent::Expired { surface_id: surface_id() })]
            }
            CardFailure::Limit(limit) if method == CARD_OPEN || method == CARD_RESUME => {
                vec![DomainEvent::Card(CardEvent::LimitExceeded {
                    surface_id: surface_id(),
                    limit,
                    resume: method == CARD_RESUME,
                })]
            }
            CardFailure::Limit(limit) => vec![refused_event(
                body,
                match limit {
                    CardLimit::Process => LIMIT_PROCESS,
                    CardLimit::Account => LIMIT_ACCOUNT,
                },
            )],
            CardFailure::Refused(reason) => vec![refused_event(body, &reason)],
            CardFailure::Other if method == CARD_SUBMIT => unknown_submit(),
            CardFailure::Other => vec![refused_event(body, "card_session_failed")],
        },
    };
    settle(method, body, &mut out);
    out
}

/// The task refusals that say the decision no longer applies to the task as
/// it is: the request changed.
const DECISION_CHANGED: [&str; 5] =
    ["stale_revision", "decision_consumed", "decision_mismatch", "terminal", "not_found"];

/// The events of the approval's grant and task decisions. Success changes
/// nothing on the card: it keeps "Waiting for confirmation" until the signed
/// update or the task change. A typed task refusal says the request changed;
/// any other failure is the card's own refusal. Nothing is retried.
fn approval_outcome(body: &Value, result: &Result<Value, BusError>) -> Vec<DomainEvent> {
    let surface_id = SurfaceId::new(body["surface_id"].as_str().unwrap_or_default());
    let Err(error) = result else { return Vec::new() };
    let changed = matches!(error, BusError::Daemon { code, reason_code, .. }
        if DECISION_CHANGED.contains(&reason_code.as_deref().unwrap_or(code.as_str())));
    if changed {
        vec![DomainEvent::Card(CardEvent::ApprovalChanged { surface_id })]
    } else {
        vec![refused_event(body, "card_session_failed")]
    }
}

/// The events of an image read: the image with its size, or "Image
/// unavailable" (whatever went wrong; only the error code is logged).
pub fn image_outcome(body: &Value, result: &Result<Value, BusError>) -> Vec<DomainEvent> {
    let surface_id = SurfaceId::new(body["surface_id"].as_str().unwrap_or_default());
    let object_id = body["object_id"].as_str().unwrap_or_default().to_owned();
    let image = match result {
        Ok(answer) => {
            answer["path"].as_str().zip(answer["width"].as_u64()).zip(answer["height"].as_u64())
        }
        Err(error) => {
            if let BusError::Daemon { code, .. } = error {
                crate::local::app_log::warn(&format!("card image refused: {code}"));
            }
            None
        }
    };
    let image = image.and_then(|((path, width), height)| {
        Some(CardImage::Ready {
            path: path.to_owned(),
            width: u32::try_from(width).ok()?,
            height: u32::try_from(height).ok()?,
        })
    });
    match image {
        Some(image) => {
            vec![DomainEvent::Card(CardEvent::ImageReady { surface_id, object_id, image })]
        }
        None => vec![DomainEvent::Card(CardEvent::ImageUnavailable { surface_id, object_id })],
    }
}

/// The events of a card call that was never sent (its body did not follow
/// the card contract): a refusal the card shows, and the settlement of a
/// host event call.
pub fn not_sent(method: &str, body: &Value) -> Vec<DomainEvent> {
    let mut out = vec![refused_event(body, "card_session_failed")];
    settle(method, body, &mut out);
    out
}

/// Ends the events of a host event call with its settlement.
fn settle(method: &str, body: &Value, out: &mut Vec<DomainEvent>) {
    if method == CARD_EVENT {
        out.push(DomainEvent::Card(CardEvent::EventSettled {
            surface_id: SurfaceId::new(body["surface_id"].as_str().unwrap_or_default()),
        }));
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
        let reason = reply.reason.as_deref().unwrap_or("card_session_refused");
        // A session with no tree is a dropped session, like an expired one.
        if EXPIRED.contains(&reason) {
            return vec![event(CardEvent::Expired { surface_id })];
        }
        return refused(reason);
    }
    let host_call = method == CARD_EVENT || method == CARD_SLOT;
    // Every event and slot reply that may be drawn carries its batch (an empty
    // one says nothing changed); one without is a reply that breaks the contract.
    if host_call && reply.render_allowed && reply.batch.is_none() {
        return refused("card_session_reply");
    }
    let batch = |b: CardBatch| Box::new(b);
    // The batch of an event or slot reply is applied first: it is the card's
    // current tree (a stale event's batch reverts the field).
    let own_batch = |reply_batch: Option<CardBatch>, draft_conflict: bool| {
        reply_batch.map(|b| {
            event(CardEvent::Batch {
                surface_id: surface_id.clone(),
                batch: batch(b),
                draft_conflict,
                own_reply: true,
            })
        })
    };
    if reply.result == CardResult::StaleDropped {
        let mut out = Vec::new();
        if host_call {
            out.extend(own_batch(reply.batch, false));
        }
        out.extend(refused(CARD_STALE_REASON));
        return out;
    }
    if reply.result == CardResult::Superseded {
        return vec![event(CardEvent::Stale { surface_id })];
    }
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
                    own_reply: false,
                }));
            }
        }
        (
            CARD_EVENT,
            CardResult::Edited
            | CardResult::Validation
            | CardResult::PendingSubmit
            | CardResult::OpenUrl,
        ) => {
            out.extend(own_batch(reply.batch, reply.draft_conflict));
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
            out.extend(own_batch(reply.batch, false));
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
            out.extend(own_batch(reply.batch, false));
            out.push(event(CardEvent::LinkClosed { surface_id: surface_id.clone() }));
        }
        (CARD_SLOT, CardResult::ApprovalDecision) => {
            let Some(HostRequest::ApprovalDecision { outcome, request_ref, task_resolve, .. }) =
                reply.host_request
            else {
                return refused("card_session_reply");
            };
            // A confirm or a deny carries the task decision core built; a
            // verdict without it breaks the contract.
            let decides = matches!(outcome, SlotOutcome::Confirm | SlotOutcome::Deny);
            if decides && task_resolve.is_none() {
                return refused("card_session_reply");
            }
            out.extend(own_batch(reply.batch, false));
            out.push(event(CardEvent::ApprovalVerdict {
                surface_id: surface_id.clone(),
                approval: CardApproval { outcome, request_ref, task_resolve },
            }));
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
const CARD_APPROVAL_FINISH: &str = ramflux_desktop_core::methods::CARD_APPROVAL_FINISH;

#[cfg(test)]
mod tests {
    use ramflux_desktop_core::{CardEvent, DomainEvent};
    use serde_json::json;

    use super::{CARD_EVENT, CARD_SUBMIT, outcome};
    use crate::bus::BusError;

    fn body() -> serde_json::Value {
        json!({ "surface_id": "s-1", "session": 4 })
    }

    fn settled(events: &[DomainEvent]) -> bool {
        matches!(events.last(), Some(DomainEvent::Card(CardEvent::EventSettled { .. })))
    }

    #[test]
    fn an_edited_reply_without_a_batch_is_a_protocol_error_and_still_settles_the_event_call() {
        let answer = json!({ "result": "edited", "render_allowed": true, "fallback_text": "" });
        let events = outcome(CARD_EVENT, &body(), &Ok(answer));
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(matches!(
            &events[0],
            DomainEvent::Card(CardEvent::Refused { reason, .. }) if reason == "card_session_reply"
        ));
        assert!(settled(&events));
    }

    #[test]
    fn every_event_call_outcome_ends_settled() {
        let refused = json!({
            "result": "refused", "reason": "card_session_refused",
            "render_allowed": true, "fallback_text": "",
        });
        for result in [
            Ok(refused),
            Ok(json!({ "result": "nonsense" })),
            Err(BusError::Timeout),
            Err(BusError::Transport("broken".into())),
            Err(BusError::daemon("Unavailable", "card runtime unavailable")),
        ] {
            let events = outcome(CARD_EVENT, &body(), &result);
            assert!(settled(&events), "{result:?} -> {events:?}");
        }
        assert!(settled(&super::not_sent(CARD_EVENT, &body())));
        assert!(!settled(&super::not_sent(CARD_SUBMIT, &body())));
    }

    #[test]
    fn a_submit_with_an_unknown_outcome_is_no_refusal() {
        for result in [
            Err(BusError::Timeout),
            Err(BusError::Transport("broken".into())),
            Ok(json!({ "result": "submitted" })),
        ] {
            let events = outcome(CARD_SUBMIT, &body(), &result);
            assert!(
                matches!(events.as_slice(), [DomainEvent::Card(CardEvent::SubmitFailed { .. })]),
                "{result:?} -> {events:?}"
            );
        }
        // A typed refusal is the card's own refusal (never retried).
        let refused = Err(BusError::Daemon {
            retry_after_ms: None,
            code: "ValidationFailed".into(),
            message: "card_session_refused".into(),
            reason_code: Some("card_session_refused".into()),
            details: None,
            cause: None,
        });
        let events = outcome(CARD_SUBMIT, &body(), &refused);
        assert!(matches!(events.as_slice(), [DomainEvent::Card(CardEvent::Refused { .. })]));
    }
}
