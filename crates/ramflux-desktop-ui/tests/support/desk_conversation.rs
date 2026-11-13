// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Bounded source facts shared by interaction and offscreen references.
use ramflux_desktop_core::{
    ContextId, DomainEvent, Lifecycle, MessagePayload, Moment, MomentId, MomentKind, ParticipantId,
    ReactionCount, Receipt, ReplyRef, Timestamp,
};
use ramflux_desktop_test::{Scenario, load_fixture};

pub fn scenario() -> Scenario {
    let mut scenario = load_fixture("s02_daily").expect("test: source scenario");
    scenario.timeline.retain(|(_, event)| !matches!(event, DomainEvent::MomentAdded(moment) if moment.context_id == ContextId::new("c-lena")));
    let mut add = |at, id: &str, origin: &str, payload| {
        scenario.timeline.push((
            at,
            DomainEvent::MomentAdded(Box::new(Moment {
                id: MomentId::new(id),
                context_id: ContextId::new("c-lena"),
                origin: ParticipantId::new(origin),
                at: Timestamp(0),
                order: None,
                lifecycle: Lifecycle::Done,
                kind: MomentKind::Message(payload),
            })),
        ));
    };
    add(
        400,
        "x-01",
        "p-lena",
        MessagePayload { text: "Check the material samples first.".into(), ..Default::default() },
    );
    add(
        450,
        "x-02",
        "p-lena",
        MessagePayload { text: "Then compare the daylight photos.".into(), ..Default::default() },
    );
    add(
        500,
        "x-03",
        "p-me",
        MessagePayload {
            text: "Agreed. <img src='never'> & **literal** 😀".into(),
            receipt: Receipt::Read,
            ..Default::default()
        },
    );
    add(
        550,
        "x-04",
        "p-orbit",
        MessagePayload {
            text: "I found three sources. Please review before sharing.".into(),
            ..Default::default()
        },
    );
    add(
        600,
        "x-05",
        "p-me",
        MessagePayload {
            text: "Here is the current sample.".into(),
            reply_to: Some(ReplyRef { message_id: MomentId::new("x-01") }),
            attachment: Some("o-moodboard".into()),
            receipt: Receipt::Delivered,
            reactions: vec![
                ReactionCount { token: "🌿".into(), count: 2, mine: true },
                ReactionCount { token: "👍".into(), count: 3, mine: false },
            ],
            revision: 1,
            ..Default::default()
        },
    );
    add(
        650,
        "x-06",
        "p-lena",
        MessagePayload { text: "removed secret".into(), tombstone: true, ..Default::default() },
    );
    add(
        700,
        "x-07",
        "p-me",
        MessagePayload {
            text: "We can review the updated source.".into(),
            reply_to: Some(ReplyRef { message_id: MomentId::new("x-06") }),
            receipt: Receipt::Sent,
            ..Default::default()
        },
    );
    scenario.timeline.push((
        2000,
        DomainEvent::MomentUpdated {
            moment_id: MomentId::new("x-01"),
            lifecycle: None,
            kind: Some(MomentKind::Message(MessagePayload {
                tombstone: true,
                ..Default::default()
            })),
        },
    ));
    scenario.timeline.push((
        3000,
        DomainEvent::MomentUpdated {
            moment_id: MomentId::new("x-03"),
            lifecycle: None,
            kind: Some(MomentKind::Message(MessagePayload {
                tombstone: true,
                ..Default::default()
            })),
        },
    ));
    scenario.timeline.sort_by_key(|(at, _)| *at);
    scenario
}
