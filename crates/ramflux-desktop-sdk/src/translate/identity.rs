// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The identity directory: participants, their verified devices and safety.
//!
//! | row | desktop |
//! |---|---|
//! | `display_name` | display (the id when null) |
//! | `kind` `human` / `headless_ai` / `bot` | `Human` / `Agent` / `Agent`; null is `Unknown` |
//! | `safety_state` `verified` / `verification_stale` | `Verified` / `KeyChanged` (the verified device set changed); else `Unverified` |
//! | `safety_number` | the persisted number, or none |
//! | `devices` | routes for sends, adds and invites (the directory) |
//! | `is_self` | this account's principal and commitment |
//!
//! Device labels and last-seen times are not reported for remote devices
//! and are never filled in; `verified_at` is not a last-seen time.

use ramflux_desktop_core::{DomainEvent, ParticipantId, ParticipantKind, SafetyState};
use serde_json::Value;

use super::Translator;
use super::contacts::Facts;
use crate::directory::{Identity, field};

fn kind(row: &Value) -> Option<ParticipantKind> {
    match row["kind"].as_str() {
        Some("human") => Some(ParticipantKind::Human),
        Some("headless_ai" | "bot") => Some(ParticipantKind::Agent),
        _ => None,
    }
}

fn safety(row: &Value) -> SafetyState {
    match row["safety_state"].as_str() {
        Some("verified") => SafetyState::Verified,
        Some("verification_stale") => SafetyState::KeyChanged,
        _ => SafetyState::Unverified,
    }
}

impl Translator {
    pub(super) fn identities(&mut self, rows: &[Value]) -> Vec<DomainEvent> {
        let mut events = Vec::new();
        let mut listed = std::collections::BTreeMap::new();
        for row in rows {
            let Some(principal) = field(row, "principal_id").or_else(|| field(row, "id")) else {
                continue;
            };
            let identity = Identity::from_row(row);
            if identity.is_self {
                self.directory.self_principal = Some(principal.clone());
                if identity.commitment.is_some() {
                    self.directory.self_commitment.clone_from(&identity.commitment);
                }
                if let Some(label) = field(row, "display_name") {
                    events.extend(self.own_label(&label));
                }
            }
            let facts = Facts {
                display: field(row, "display_name"),
                kind: kind(row),
                safety: safety(row),
                safety_number: field(row, "safety_number"),
            };
            events.extend(self.set_facts(&ParticipantId::new(principal.as_str()), facts));
            listed.insert(principal, identity);
        }
        self.directory.identities = listed;
        events
    }
}
