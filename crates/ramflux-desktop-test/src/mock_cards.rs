// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Card answers of the demo (mock) source. The demo surfaces that carry a
//! card answer `card.session.open` with the card runtime's tree for the
//! surface, taken from the copies of the core's host fixtures
//! (`card-fixtures/`, pinned by `scripts/sync-card-fixtures.sh`). The demo
//! runs no runtime: a host event, slot answer, submit or update is refused,
//! and the card keeps its tree. Surfaces without a fixture refuse the open,
//! so they show their fallback text.

use std::sync::atomic::{AtomicU64, Ordering};

use ramflux_desktop_core::{CardBatch, CardEvent, DomainEvent, SurfaceId};
use serde_json::{Value, json};

/// The fixture tree each demo surface draws.
fn fixture(surface: &str) -> Option<&'static str> {
    match surface {
        "s-chart" => Some(include_str!("../../../card-fixtures/fixtures/chart.tree.json")),
        "s-plan" => Some(include_str!("../../../card-fixtures/fixtures/task.tree.json")),
        "s-window" => Some(include_str!("../../../card-fixtures/fixtures/form.tree.json")),
        _ => None,
    }
}

/// Session identifiers of the demo runtime (one per open).
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

/// The events one card call means in the demo.
#[must_use]
pub fn respond(method: &str, body: &Value) -> Vec<DomainEvent> {
    let surface = body["surface_id"].as_str().unwrap_or_default();
    let surface_id = SurfaceId::new(surface);
    match method {
        "card.session.open" | "card.session.resume" => match fixture(surface).and_then(open) {
            Some((session, batch)) if method.ends_with("open") => {
                vec![DomainEvent::Card(CardEvent::Opened {
                    surface_id,
                    session,
                    batch: Box::new(batch),
                })]
            }
            Some((session, batch)) => vec![DomainEvent::Card(CardEvent::Resumed {
                surface_id,
                session,
                batch: Box::new(batch),
            })],
            None => vec![DomainEvent::Card(CardEvent::Refused {
                surface_id,
                reason: "card_session_refused".to_owned(),
            })],
        },
        "card.session.present" => Vec::new(),
        "card.session.suspend" | "card.session.dispose" => Vec::new(),
        // A host event call always settles, so the card takes the next one.
        "card.session.event" => vec![
            DomainEvent::Card(CardEvent::Refused {
                surface_id: surface_id.clone(),
                reason: "card_session_refused".to_owned(),
            }),
            DomainEvent::Card(CardEvent::EventSettled { surface_id }),
        ],
        _ => vec![DomainEvent::Card(CardEvent::Refused {
            surface_id,
            reason: "card_session_refused".to_owned(),
        })],
    }
}

/// The open batch (a full tree) and its session for a fixture tree.
fn open(tree: &str) -> Option<(u64, CardBatch)> {
    let session = NEXT_SESSION.fetch_add(1, Ordering::SeqCst);
    let mut tree: Value = serde_json::from_str(tree).ok()?;
    rewrite_session(&mut tree, session);
    let nodes = tree["nodes"].as_array()?;
    let mut mutations = Vec::with_capacity(nodes.len());
    for node in nodes {
        let parent = node["parent_key"].as_str();
        let index = parent
            .and_then(|parent| nodes.iter().find(|row| row["key"].as_str() == Some(parent)))
            .and_then(|row| row["children"].as_array())
            .and_then(|children| children.iter().position(|child| child == &node["key"]))
            .unwrap_or(0);
        mutations.push(json!({"op": "upsert", "parent": parent, "index": index, "node": node}));
    }
    let batch = json!({
        "schema": ramflux_desktop_core::MUTATIONS_SCHEMA,
        "session": session,
        "previous_version": 0,
        "version": tree["tree_version"],
        "signed_version": tree["signed_version"],
        "signed_hash": tree["signed_hash"],
        "edit_seq": tree["edit_seq"],
        "mutations": mutations,
        "host_slots": tree["host_slots"],
        "locale": tree["locale"],
        "format_locale": tree["format_locale"],
        "direction": tree["direction"],
    });
    let batch: CardBatch = serde_json::from_value(batch).ok()?;
    Some((session, batch))
}

/// Points every session field of the fixture (and of its host slot tokens) at
/// the demo session.
fn rewrite_session(value: &mut Value, session: u64) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if key == "session" {
                    *child = json!(session);
                } else {
                    rewrite_session(child, session);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| rewrite_session(item, session)),
        _ => {}
    }
}
