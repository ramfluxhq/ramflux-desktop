// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A v1 call control (content kind `Call`): all 13 actions.
//!
//! Each control is typed: [`signal_of`] maps the generated action to a
//! [`Signal`] and nothing else is read. Key material (`base_key`), lease
//! consent and the peer allocation never leave this module; only a rekey's
//! generation number is kept. The call's state follows the receiver's view
//! of the signalling:
//!
//! - `invite` places the call (ringing, by the local side); `answer` and
//!   `ack` connect it; `ready` makes it active;
//! - `reject`, `busy`, `hangup`, `timeout`, `failed` and `answered_elsewhere`
//!   end it, with the reason the local side would show;
//! - `ringing`, `rekey` and `rekey_ack` leave the state alone; a rekey
//!   asks for a higher generation and its ack answers exactly that one.
//!
//! Who may send what is checked: a control must come from the caller or the
//! callee of its call, and an invite only from the caller. A control at or
//! below the last sequence of its sender is a replay; a call that ended
//! never revives.
//!
//! Capacity: at most [`MAX_CALLS`] calls are live. An ended call leaves the
//! live set at once and is remembered in a bounded history of
//! [`MAX_RETIRED`] ids; when the history evicts an id, its invite time joins
//! a floor, and an invite at or below the floor is refused. So finished
//! calls never use up live capacity, and an ended call, retired or evicted,
//! cannot be placed again by replaying its invite. A call id belongs to
//! one conversation (call ids are account-wide upstream): a control that
//! names another conversation or other endpoints is refused. Everything is
//! dropped on lock.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use ramflux_desktop_core::{
    Call, CallDirection, CallEndReason, CallId, CallState, DomainEvent, ParticipantId, Timestamp,
};
use ramflux_protocol::message::{
    Call as WireCall, CallAction, CallEndpoint, EventIdentity, call_action::Value as Action,
};

use super::{context_of, order_of};
use crate::translate::Translator;

/// Live calls retained per account.
pub const MAX_CALLS: usize = 256;

/// Ended calls remembered by id.
pub const MAX_RETIRED: usize = 512;

/// What a control says, without any key material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// The caller places the call.
    Invite,
    /// The callee's device rings.
    Ringing,
    /// The callee answers.
    Answer,
    /// The caller acknowledges the answer with a media allocation.
    Ack,
    /// Media is ready.
    Ready,
    /// The callee declines.
    Reject,
    /// The callee is in another call.
    Busy,
    /// Either side hangs up.
    Hangup,
    /// The ring timed out.
    Timeout,
    /// Signalling or media failed.
    Failed,
    /// Another device of the callee's account answered.
    AnsweredElsewhere,
    /// A new media generation is proposed.
    Rekey {
        /// The proposed generation.
        generation: u64,
    },
    /// A proposed generation is accepted.
    RekeyAck {
        /// The accepted generation.
        generation: u64,
    },
}

/// The signal of an action, or `None` for an action with no arm.
#[must_use]
pub fn signal_of(action: &CallAction) -> Option<Signal> {
    Some(match action.value.as_ref()? {
        Action::Invite(_) => Signal::Invite,
        Action::Ringing(_) => Signal::Ringing,
        Action::Answer(_) => Signal::Answer,
        Action::Ack(_) => Signal::Ack,
        Action::Ready(_) => Signal::Ready,
        Action::Reject(_) => Signal::Reject,
        Action::Busy(_) => Signal::Busy,
        Action::Hangup(_) => Signal::Hangup,
        Action::Timeout(_) => Signal::Timeout,
        Action::Failed(_) => Signal::Failed,
        Action::AnsweredElsewhere(_) => Signal::AnsweredElsewhere,
        Action::Rekey(rekey) => Signal::Rekey { generation: rekey.generation },
        Action::RekeyAck(ack) => Signal::RekeyAck { generation: ack.generation },
    })
}

/// What is kept of one call.
#[derive(Debug, Clone)]
struct Known {
    call: Call,
    caller: String,
    callee: String,
    /// The last sequence number of each endpoint's controls.
    seen: BTreeMap<String, u64>,
    requested: u64,
    acknowledged: u64,
    /// When the invite was made, in seconds.
    invited_at: i64,
}

/// The v1 calls seen.
#[derive(Debug, Default)]
pub struct Book {
    calls: BTreeMap<CallId, Known>,
    /// Ended calls, oldest first, with their invite times.
    retired: VecDeque<(CallId, i64)>,
    retired_ids: BTreeSet<CallId>,
    /// Invites at or below this time are never placed (their calls ended
    /// and were forgotten).
    floor: i64,
}

impl Book {
    /// Forgets everything (lock, account switch).
    pub fn clear(&mut self) {
        self.calls.clear();
        self.retired.clear();
        self.retired_ids.clear();
        self.floor = 0;
    }

    /// Live calls.
    #[must_use]
    pub fn len(&self) -> usize {
        self.calls.len()
    }

    /// Whether no call is live.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    /// Ended calls remembered by id.
    #[must_use]
    pub fn retired(&self) -> usize {
        self.retired.len()
    }

    fn retire(&mut self, id: CallId, invited_at: i64) {
        self.calls.remove(&id);
        self.retired_ids.insert(id.clone());
        self.retired.push_back((id, invited_at));
        while self.retired.len() > MAX_RETIRED {
            if let Some((old, at)) = self.retired.pop_front() {
                self.retired_ids.remove(&old);
                self.floor = self.floor.max(at);
            }
        }
    }
}

/// The media generations of a call: proposed and acknowledged.
#[cfg(feature = "test-support")]
#[must_use]
pub fn generations(translator: &Translator, call: &CallId) -> Option<(u64, u64)> {
    translator.v1_calls.calls.get(call).map(|known| (known.requested, known.acknowledged))
}

/// Live and retired calls (tests only).
#[cfg(feature = "test-support")]
#[must_use]
pub fn counts(translator: &Translator) -> (usize, usize) {
    (translator.v1_calls.len(), translator.v1_calls.retired())
}

/// Sets the principal the translator acts for (tests only).
#[cfg(feature = "test-support")]
pub fn set_self_principal(translator: &mut Translator, principal: &str) {
    translator.directory.self_principal = Some(principal.to_owned());
}

fn endpoint(endpoint: Option<&CallEndpoint>) -> Option<&str> {
    endpoint.map(|e| e.principal_id.as_str()).filter(|id| !id.is_empty())
}

pub(super) fn project(
    translator: &mut Translator,
    identity: &EventIdentity,
    call: &WireCall,
) -> Vec<DomainEvent> {
    let Some(control) = &call.control else { return Vec::new() };
    let (Some(caller), Some(callee), Some(sender)) = (
        endpoint(control.caller.as_ref()),
        endpoint(control.callee.as_ref()),
        endpoint(control.sender.as_ref()),
    ) else {
        return Vec::new();
    };
    let Some(signal) = control.body.as_ref().and_then(signal_of) else { return Vec::new() };
    let Some(local) = translator.directory.self_principal.clone() else { return Vec::new() };
    let context = context_of(identity);
    if control.call_id.is_empty()
        || control.seq == 0
        || control.conversation_id != identity.conversation_id
        || caller == callee
        || (local != caller && local != callee)
        || (sender != caller && sender != callee)
        || (signal == Signal::Invite && sender != caller)
    {
        return Vec::new();
    }
    let id = CallId::new(&control.call_id);
    let at = Timestamp(control.created_at.saturating_mul(1000));
    let incoming = local == callee;
    let book = &mut translator.v1_calls;
    // An ended call stays ended, whatever arrives for it.
    if book.retired_ids.contains(&id) {
        return Vec::new();
    }
    let Some(known) = book.calls.get_mut(&id) else {
        // A call is placed by its invite; any other first control cannot be
        // placed (nothing is guessed).
        if signal != Signal::Invite
            || book.calls.len() >= MAX_CALLS
            || control.created_at <= book.floor
        {
            return Vec::new();
        }
        let call = Call {
            id: id.clone(),
            context_id: context,
            peer: ParticipantId::new(if incoming { caller } else { callee }),
            caller: ParticipantId::new(caller),
            direction: if incoming { CallDirection::Incoming } else { CallDirection::Outgoing },
            state: if incoming { CallState::IncomingRinging } else { CallState::OutgoingRinging },
            version: 1,
            started_at: at,
            answered_at: None,
            ended_at: None,
            order: Some(order_of(identity)),
        };
        let known = Known {
            call: call.clone(),
            caller: caller.to_owned(),
            callee: callee.to_owned(),
            seen: BTreeMap::from([(sender.to_owned(), control.seq)]),
            requested: 0,
            acknowledged: 0,
            invited_at: control.created_at,
        };
        book.calls.insert(id, known);
        return vec![DomainEvent::CallUpserted(call)];
    };
    // One call, one conversation and one pair of endpoints.
    if known.call.context_id != context || known.caller != caller || known.callee != callee {
        return Vec::new();
    }
    if known.seen.get(sender).is_some_and(|last| control.seq <= *last) {
        return Vec::new();
    }
    known.seen.insert(sender.to_owned(), control.seq);
    let next = next_state(known, signal, sender, incoming);
    match signal {
        Signal::Rekey { generation } => {
            let under_way = matches!(known.call.state, CallState::Connecting | CallState::Active);
            if under_way && generation > known.requested.max(known.acknowledged) {
                known.requested = generation;
            }
            return Vec::new();
        }
        Signal::RekeyAck { generation } => {
            let under_way = matches!(known.call.state, CallState::Connecting | CallState::Active);
            if under_way && generation == known.requested && generation > known.acknowledged {
                known.acknowledged = generation;
            }
            return Vec::new();
        }
        _ => {}
    }
    let Some(state) = next.filter(|state| *state != known.call.state) else { return Vec::new() };
    if !CallState::can_transition(known.call.state, state) {
        return Vec::new();
    }
    known.call.state = state;
    known.call.version += 1;
    match state {
        CallState::Connecting => known.call.answered_at = Some(at),
        CallState::Ended(_) => known.call.ended_at = Some(at),
        _ => {}
    }
    let event = DomainEvent::CallUpserted(known.call.clone());
    if state.is_ended() {
        let invited_at = known.invited_at;
        book.retire(id, invited_at);
    }
    vec![event]
}

/// The state `signal` from `sender` leads to, as the local side shows it.
fn next_state(known: &Known, signal: Signal, sender: &str, incoming: bool) -> Option<CallState> {
    use CallEndReason as End;
    let state = known.call.state;
    let ringing = matches!(state, CallState::OutgoingRinging | CallState::IncomingRinging);
    let answered = matches!(state, CallState::Connecting | CallState::Active);
    let from_caller = sender == known.caller;
    Some(match signal {
        Signal::Invite | Signal::Ringing | Signal::Rekey { .. } | Signal::RekeyAck { .. } => {
            return None;
        }
        Signal::Answer if ringing && !from_caller => CallState::Connecting,
        Signal::Ack if answered && from_caller => CallState::Connecting,
        Signal::Ready if state == CallState::Connecting => CallState::Active,
        Signal::Reject if ringing && !from_caller => CallState::Ended(End::Declined),
        Signal::Busy if ringing && !from_caller => CallState::Ended(End::Busy),
        Signal::Hangup if answered => CallState::Ended(End::Completed),
        Signal::Hangup if ringing && !from_caller => CallState::Ended(End::Declined),
        Signal::Hangup if ringing && incoming => CallState::Ended(End::Missed),
        Signal::Hangup if ringing => CallState::Ended(End::Cancelled),
        Signal::Timeout if ringing && incoming => CallState::Ended(End::Missed),
        Signal::Timeout if ringing => CallState::Ended(End::Timeout),
        Signal::Failed if !state.is_ended() => CallState::Ended(End::Failed),
        Signal::AnsweredElsewhere if state == CallState::IncomingRinging && from_caller => {
            CallState::Ended(End::AnsweredElsewhere)
        }
        _ => return None,
    })
}
