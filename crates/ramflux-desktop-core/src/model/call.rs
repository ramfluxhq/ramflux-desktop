// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! One-to-one voice calls (CALL-01 §2 and §6).
//!
//! The daemon owns a call's signaling and reports each change as the whole
//! call (`call.changed`), with a per-call sequence number that rises. The
//! store keeps the call and derives its record in the conversation
//! timeline (one [`super::MomentKind::Call`] moment per call), so a call is
//! never shown twice and its record always matches its state.
//!
//! The media of a call (where to send, the relay token, the SFrame keys)
//! is not part of the call: it is secret, never enters an event or the
//! store, and travels from the source to the media engine only
//! ([`CallMedia`]).

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::moment::Lifecycle;
use crate::ids::{CallId, ContextId, ParticipantId, Timestamp};

/// Who placed the call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CallDirection {
    /// This account called.
    Outgoing,
    /// The peer called.
    Incoming,
}

/// Why a call ended, as the daemon reports it from this account's side
/// (`end_reason`: `completed`, `cancelled`, `declined`, `busy`, `timeout`,
/// `answered_elsewhere`, `failed`, `missed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CallEndReason {
    /// An answered call was hung up (either side).
    Completed,
    /// The caller hung up before an answer (the caller's view).
    Cancelled,
    /// The callee declined.
    Declined,
    /// The callee was in another call.
    Busy,
    /// Nobody answered within the ring timeout, 45 s (the caller's view).
    Timeout,
    /// Another device of this account answered.
    AnsweredElsewhere,
    /// Signaling or media failed, or the daemon restarted, or a device or
    /// key changed during the call.
    Failed,
    /// The callee's view of a call that timed out or was cancelled.
    Missed,
}

/// Where a call is (CALL-01 §6: `outgoing_ringing`, `incoming_ringing`,
/// `connecting`, `active`, `ended{reason}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CallState {
    /// This account is calling; the peer's devices ring.
    OutgoingRinging,
    /// The peer is calling; this device rings.
    IncomingRinging,
    /// Answered; media is being set up.
    Connecting,
    /// Media flows.
    Active,
    /// Over. Terminal.
    Ended(CallEndReason),
}

impl CallState {
    /// Whether the call is over.
    pub const fn is_ended(self) -> bool {
        matches!(self, Self::Ended(_))
    }

    /// Whether `from -> to` is a legal transition. Self-transitions are not
    /// transitions and return false.
    pub const fn can_transition(from: Self, to: Self) -> bool {
        use CallState::{Active, Connecting, Ended, IncomingRinging, OutgoingRinging};
        matches!(
            (from, to),
            (OutgoingRinging | IncomingRinging, Connecting | Ended(_))
                | (Connecting, Active | Ended(_))
                | (Active, Ended(_))
        )
    }

    /// The lifecycle of the call's timeline record: a ringing incoming call
    /// needs the user, a call under way is in progress, an ended one done.
    pub const fn moment_lifecycle(self) -> Lifecycle {
        match self {
            Self::IncomingRinging => Lifecycle::NeedsMe,
            Self::OutgoingRinging | Self::Connecting | Self::Active => Lifecycle::InProgress,
            Self::Ended(_) => Lifecycle::Done,
        }
    }
}

/// How an ended call reads in the timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallOutcome {
    /// It was answered; how long it lasted.
    Answered {
        /// Milliseconds from answer to end.
        duration_ms: u64,
    },
    /// An incoming call nobody answered here (cancelled, timed out, or
    /// this account was busy).
    Missed,
    /// Declined (by the user here, or by the peer).
    Declined,
    /// The peer did not answer an outgoing call.
    NoAnswer,
    /// The peer was in another call.
    Busy,
    /// This account cancelled its own call before an answer.
    Cancelled,
    /// Another device of this account took it.
    AnsweredElsewhere,
    /// It failed.
    Failed,
}

/// One voice call, as the daemon last reported it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    /// The call id (the daemon's, fresh per call).
    pub id: CallId,
    /// The direct conversation it belongs to.
    pub context_id: ContextId,
    /// The other participant.
    pub peer: ParticipantId,
    /// Who placed it (the moment's origin): this account or the peer.
    pub caller: ParticipantId,
    /// Who placed it.
    pub direction: CallDirection,
    /// Where it is.
    pub state: CallState,
    /// The daemon's per-call sequence number; rises with every change.
    pub version: u64,
    /// When it was placed.
    pub started_at: Timestamp,
    /// When it was answered.
    #[serde(default)]
    pub answered_at: Option<Timestamp>,
    /// When it ended.
    #[serde(default)]
    pub ended_at: Option<Timestamp>,
}

impl Call {
    /// The timeline record's moment id (one per call).
    pub fn moment_id(id: &CallId) -> crate::ids::MomentId {
        crate::ids::MomentId::new(format!("call:{id}"))
    }

    /// How the call reads once ended; `None` while it is under way.
    pub fn outcome(&self) -> Option<CallOutcome> {
        let CallState::Ended(reason) = self.state else { return None };
        if let (Some(answered), Some(ended)) = (self.answered_at, self.ended_at) {
            return Some(CallOutcome::Answered { duration_ms: ended.saturating_since(answered) });
        }
        if self.answered_at.is_some() {
            return Some(CallOutcome::Answered { duration_ms: 0 });
        }
        let incoming = self.direction == CallDirection::Incoming;
        Some(match reason {
            CallEndReason::AnsweredElsewhere => CallOutcome::AnsweredElsewhere,
            CallEndReason::Declined => CallOutcome::Declined,
            CallEndReason::Failed => CallOutcome::Failed,
            CallEndReason::Missed => CallOutcome::Missed,
            CallEndReason::Completed
            | CallEndReason::Cancelled
            | CallEndReason::Timeout
            | CallEndReason::Busy
                if incoming =>
            {
                CallOutcome::Missed
            }
            CallEndReason::Completed | CallEndReason::Cancelled => CallOutcome::Cancelled,
            CallEndReason::Timeout => CallOutcome::NoAnswer,
            CallEndReason::Busy => CallOutcome::Busy,
        })
    }
}

/// One direction's SFrame key: its key id and base key. Wiped on drop,
/// never printed, never serialized.
pub struct CallKey {
    kid: u64,
    base_key: Zeroizing<Vec<u8>>,
}

impl CallKey {
    /// A key for `kid`.
    pub fn new(kid: u64, base_key: Vec<u8>) -> Self {
        Self { kid, base_key: Zeroizing::new(base_key) }
    }

    /// The key id.
    pub fn kid(&self) -> u64 {
        self.kid
    }

    /// The base key bytes.
    pub fn base_key(&self) -> &[u8] {
        &self.base_key
    }
}

impl std::fmt::Debug for CallKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallKey").field("kid", &self.kid).finish_non_exhaustive()
    }
}

/// What lets this side move its relay binding to a new address (CALL-01
/// r2 rebinding): the lease, which side of it this device is, and this
/// side's private rebind key. Secret like the rest of [`CallMedia`].
pub struct CallRebind {
    /// The relay lease id (16 bytes).
    pub lease_id: [u8; 16],
    /// This device's side of the lease: 0 the caller, 1 the callee.
    pub side: u8,
    /// This side's rebind key (32 bytes, HMAC-SHA256 key). Wiped on drop.
    pub key: Zeroizing<Vec<u8>>,
}

impl std::fmt::Debug for CallRebind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallRebind").field("side", &self.side).finish_non_exhaustive()
    }
}

/// The media session of a call (`call.media.session`): where to send, this
/// side's relay allocation token and rebind credential, the SFrame suite
/// and both directions' keys. Secret: it goes from the source to the media
/// engine and nowhere else, and the daemon hands it out once per call.
pub struct CallMedia {
    /// The call.
    pub call_id: CallId,
    /// The relay allocation, `host:port`.
    pub endpoint: String,
    /// The opaque allocation token (empty: a direct endpoint).
    pub token: Zeroizing<Vec<u8>>,
    /// The relay rebinding credential (`None`: a direct endpoint).
    pub rebind: Option<CallRebind>,
    /// The SFrame cipher suite (IANA id).
    pub suite: u16,
    /// This device's sending key.
    pub send: CallKey,
    /// The peer's sending key.
    pub receive: CallKey,
}

impl std::fmt::Debug for CallMedia {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallMedia")
            .field("call_id", &self.call_id)
            .field("endpoint", &self.endpoint)
            .field("rebind", &self.rebind)
            .field("suite", &self.suite)
            .field("send", &self.send)
            .field("receive", &self.receive)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_table() {
        use CallState::*;
        let ended = Ended(CallEndReason::Completed);
        let all = [OutgoingRinging, IncomingRinging, Connecting, Active, ended];
        let allowed = [
            (OutgoingRinging, Connecting),
            (OutgoingRinging, ended),
            (IncomingRinging, Connecting),
            (IncomingRinging, ended),
            (Connecting, Active),
            (Connecting, ended),
            (Active, ended),
        ];
        for from in all {
            for to in all {
                assert_eq!(
                    CallState::can_transition(from, to),
                    allowed.contains(&(from, to)),
                    "{from:?} -> {to:?}"
                );
            }
        }
        assert!(!CallState::can_transition(ended, Ended(CallEndReason::Busy)));
    }

    fn call(direction: CallDirection, reason: CallEndReason, answered: bool) -> Call {
        Call {
            id: CallId::new("c"),
            context_id: ContextId::new("ctx"),
            peer: ParticipantId::new("p"),
            caller: ParticipantId::new("p"),
            direction,
            state: CallState::Ended(reason),
            version: 3,
            started_at: Timestamp(1_000),
            answered_at: answered.then_some(Timestamp(2_000)),
            ended_at: Some(Timestamp(65_000)),
        }
    }

    #[test]
    fn outcomes_read_from_the_side_of_this_account() {
        use CallDirection::{Incoming, Outgoing};
        use CallEndReason::*;
        let answered = CallOutcome::Answered { duration_ms: 63_000 };
        assert_eq!(call(Outgoing, Completed, true).outcome(), Some(answered));
        assert_eq!(call(Incoming, Failed, true).outcome(), Some(answered));
        assert_eq!(call(Incoming, Missed, false).outcome(), Some(CallOutcome::Missed));
        assert_eq!(call(Incoming, Cancelled, false).outcome(), Some(CallOutcome::Missed));
        assert_eq!(call(Incoming, Timeout, false).outcome(), Some(CallOutcome::Missed));
        assert_eq!(call(Incoming, Declined, false).outcome(), Some(CallOutcome::Declined));
        assert_eq!(call(Incoming, Failed, false).outcome(), Some(CallOutcome::Failed));
        assert_eq!(
            call(Incoming, AnsweredElsewhere, false).outcome(),
            Some(CallOutcome::AnsweredElsewhere)
        );
        assert_eq!(call(Outgoing, Cancelled, false).outcome(), Some(CallOutcome::Cancelled));
        assert_eq!(call(Outgoing, Timeout, false).outcome(), Some(CallOutcome::NoAnswer));
        assert_eq!(call(Outgoing, Busy, false).outcome(), Some(CallOutcome::Busy));
        assert_eq!(call(Outgoing, Declined, false).outcome(), Some(CallOutcome::Declined));
        let mut ringing = call(Outgoing, Cancelled, false);
        ringing.state = CallState::OutgoingRinging;
        assert_eq!(ringing.outcome(), None);
    }

    #[test]
    fn media_never_prints_its_secrets() {
        let media = CallMedia {
            call_id: CallId::new("c"),
            endpoint: "relay:3478".to_owned(),
            token: Zeroizing::new(b"token-bytes".to_vec()),
            rebind: Some(CallRebind {
                lease_id: [0x5a; 16],
                side: 1,
                key: Zeroizing::new(vec![0xef; 32]),
            }),
            suite: 5,
            send: CallKey::new(1, vec![0xab; 32]),
            receive: CallKey::new(2, vec![0xcd; 32]),
        };
        let printed = format!("{media:?}");
        assert!(!printed.contains("token-bytes") && !printed.contains("171"), "{printed}");
        assert!(!printed.contains("239") && !printed.contains("90,"), "{printed}");
        assert!(printed.contains("kid: 1"));
    }
}
