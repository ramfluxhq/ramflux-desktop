// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Generated large scenarios for scale tests (too big to keep as RON files).

use std::collections::BTreeMap;

use ramflux_desktop_core::{
    Account, AccountId, AccountKind, Context, ContextId, ContextKind, DomainEvent, Lifecycle,
    Listing, LockState, MessagePayload, Moment, MomentId, MomentKind, Participant, ParticipantId,
    ParticipantKind, Receipt, SafetyState, SourceStatus, Timestamp,
};

use crate::scenario::{DEFAULT_EPOCH, Scenario};

/// The context of [`large_context`].
pub const LARGE_CONTEXT: &str = "c-large";

/// Id of message `n` (zero-based, oldest first) of [`large_context`].
pub fn large_message_id(n: usize) -> MomentId {
    MomentId::new(format!("m-large-{n:06}"))
}

fn participant(id: &str, display: &str) -> DomainEvent {
    DomainEvent::ParticipantUpserted(Participant {
        id: ParticipantId::new(id),
        kind: ParticipantKind::Human,
        display: display.to_owned(),
        safety: SafetyState::Verified,
        contact: true,
        safety_number: None,
    })
}

/// One pinned direct context holding `messages` messages of history, a
/// minute apart and ending a minute before the epoch, all delivered at
/// offset 0. Every collection is listed.
pub fn large_context(messages: usize) -> Scenario {
    let me = ParticipantId::new("p-me");
    let ana = ParticipantId::new("p-ana");
    let mut timeline = vec![
        (0, DomainEvent::SourceStatusChanged(SourceStatus::Connected)),
        (0, DomainEvent::ListingsLoaded(Listing::ALL.to_vec())),
        (
            0,
            DomainEvent::AccountUpdated(Account {
                id: AccountId::new("local"),
                label: "Local".to_owned(),
                kind: AccountKind::HumanInteractive,
                node_id: None,
                device_id: None,
                lock_state: LockState::Unlocked,
                self_participant_id: Some(me.clone()),
            }),
        ),
        (0, participant("p-me", "Alex Moreau")),
        (0, participant("p-ana", "Ana Lima")),
        (
            0,
            DomainEvent::ContextUpserted(Context {
                id: ContextId::new(LARGE_CONTEXT),
                kind: ContextKind::Direct,
                title: "Ana Lima".to_owned(),
                participants: vec![me.clone(), ana.clone()],
                pinned: true,
                unread: 0,
                muted: false,
                disappearing_secs: None,
                roles: BTreeMap::new(),
                epoch: None,
            }),
        ),
    ];
    let count = i64::try_from(messages).unwrap_or(i64::MAX);
    timeline.extend((0..messages).map(|n| {
        let index = i64::try_from(n).unwrap_or(0);
        // Signed offsets from the epoch: history ending a minute ago.
        let at = Timestamp((index - count) * 60_000);
        let origin = if n % 3 == 0 { me.clone() } else { ana.clone() };
        let moment = Moment {
            id: large_message_id(n),
            context_id: ContextId::new(LARGE_CONTEXT),
            origin,
            at,
            kind: MomentKind::Message(MessagePayload {
                text: format!("Message {n}: a line of ordinary chat so rows have real text."),
                receipt: Receipt::Read,
                tombstone: false,
                attachment: None,
                franking: None,
            }),
            lifecycle: Lifecycle::Done,
        };
        (0, DomainEvent::MomentAdded(Box::new(moment)))
    }));
    Scenario {
        name: format!("large_context_{messages}"),
        description: format!("One context with {messages} messages"),
        epoch: DEFAULT_EPOCH,
        timeline,
        responses: Vec::new(),
        companions: Vec::new(),
    }
}
