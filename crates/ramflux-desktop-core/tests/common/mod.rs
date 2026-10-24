// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Builders shared by the core integration tests.

#![allow(dead_code)]

use ramflux_desktop_core::*;

/// Feeds events into a store with increasing sequence numbers.
pub struct Feed {
    pub store: Store,
    pub seq: u64,
}

impl Feed {
    pub fn new() -> Self {
        Self { store: Store::new(), seq: 0 }
    }

    pub fn push(&mut self, at: i64, event: DomainEvent) -> Result<Changes, ApplyError> {
        self.seq += 1;
        self.store.apply(&EventEnvelope::new(self.seq, Timestamp(at), event))
    }

    /// Pushes an event that must apply cleanly.
    pub fn ok(&mut self, at: i64, event: DomainEvent) -> Changes {
        match self.push(at, event) {
            Ok(changes) => changes,
            Err(error) => panic!("unexpected apply error: {error}"),
        }
    }
}

pub fn context(id: &str, pinned: bool) -> Context {
    Context {
        id: ContextId::new(id),
        kind: ContextKind::Group,
        title: format!("Context {id}"),
        participants: Vec::new(),
        pinned,
        unread: 0,
        muted: false,
        disappearing_secs: None,
        roles: std::collections::BTreeMap::new(),
        epoch: None,
    }
}

pub fn message(text: &str) -> MomentKind {
    MomentKind::Message(MessagePayload {
        text: text.to_owned(),
        receipt: Receipt::Sent,
        tombstone: false,
        attachment: None,
        franking: None,
    })
}

pub fn moment(id: &str, ctx: &str, at: i64, kind: MomentKind, lifecycle: Lifecycle) -> Moment {
    Moment {
        id: MomentId::new(id),
        context_id: ContextId::new(ctx),
        origin: ParticipantId::new("p-ada"),
        at: Timestamp(at),
        kind,
        lifecycle,
    }
}

pub fn added(m: Moment) -> DomainEvent {
    DomainEvent::MomentAdded(Box::new(m))
}

pub fn grant(id: &str, risk: Risk) -> Grant {
    Grant {
        id: GrantId::new(id),
        scope: "tool.shell".to_owned(),
        risk,
        single_use: risk.requires_confirmation(),
        args_hash: format!("hash-{id}"),
        expiry: None,
        state: GrantState::Pending,
        failure: None,
    }
}

pub fn participant(id: &str, kind: ParticipantKind, display: &str, contact: bool) -> Participant {
    Participant {
        id: ParticipantId::new(id),
        kind,
        display: display.to_owned(),
        safety: SafetyState::Unverified,
        contact,
        safety_number: None,
        link: ramflux_desktop_core::ContactLink::None,
        reachable: None,
        kind_asserted: false,
        root_source: None,
    }
}

pub fn set_lifecycle(id: &str, lifecycle: Lifecycle) -> DomainEvent {
    DomainEvent::MomentUpdated {
        moment_id: MomentId::new(id),
        lifecycle: Some(lifecycle),
        kind: None,
    }
}

pub fn surface(id: &str) -> MomentKind {
    MomentKind::AgentSurface(AgentSurfacePayload {
        surface_id: SurfaceId::new(id),
        fallback_text: "Agent output".to_owned(),
        blocks: vec![Block::new(
            BlockId::new("b-text"),
            BlockKind::Text,
            serde_json::json!({ "text": "Hello" }),
        )],
        actions: vec![
            SurfaceAction {
                id: "run".to_owned(),
                label: "Run".to_owned(),
                risk: Risk::Low,
                disabled_reason: None,
            },
            SurfaceAction {
                id: "export".to_owned(),
                label: "Export".to_owned(),
                risk: Risk::Medium,
                disabled_reason: Some("export is disabled on this device".to_owned()),
            },
        ],
        streaming: true,
    })
}
