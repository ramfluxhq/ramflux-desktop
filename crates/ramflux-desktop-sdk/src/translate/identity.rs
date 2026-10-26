// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The identity directory: participants, their verified devices and safety.
//!
//! | row | desktop |
//! |---|---|
//! | `display_name` | display (the id when null; a verified profile's name once one is held) |
//! | `kind` `human` / `headless_ai` / `bot` with `kind_verified` | the kind claim: `Verified` by a root-signed profile, else `Asserted` (a legacy device-set kind, a card); null is `None`, the kind `Unknown` |
//! | `display_name_confusable_with`, `mixed_script` | the name warnings |
//! | `safety_state` `verified` / `verification_stale` | `Verified` / `VerificationStale` (the manually verified authority changed: verify again); else `Unverified` |
//! | `safety_number` | the persisted number (a string, or the daemon's digit groups joined by spaces), or none |
//! | `root_source` `lookup` / `card` / `verified` | where the pinned root came from |
//! | `devices` | routes for sends, adds and invites (the directory) |
//! | `is_self` | this account's principal and commitment |
//!
//! Device labels and last-seen times are not reported for remote devices
//! and are never filled in; `verified_at` is not a last-seen time.

use ramflux_desktop_core::{DomainEvent, ParticipantId, SafetyState};
use serde_json::Value;

use super::Translator;
use super::contacts::{Facts, kind_claim, name_warnings};
use crate::directory::{Identity, field};

/// The persisted safety number: the daemon stores the digit groups it
/// computed (an array of strings); a single string is taken as it is.
pub(super) fn safety_number(row: &Value) -> Option<String> {
    match &row["safety_number"] {
        Value::Array(groups) => {
            let groups: Vec<&str> =
                groups.iter().filter_map(Value::as_str).filter(|g| !g.is_empty()).collect();
            (!groups.is_empty()).then(|| groups.join(" "))
        }
        _ => field(row, "safety_number"),
    }
}

fn safety(row: &Value) -> SafetyState {
    match row["safety_state"].as_str() {
        Some("verified") => SafetyState::Verified,
        Some("verification_stale") => SafetyState::VerificationStale,
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
                claim: kind_claim(row, "kind"),
                warnings: name_warnings(row),
                safety: safety(row),
                safety_number: safety_number(row),
                root_source: super::contacts::root_source(row),
            };
            events.extend(self.set_facts(&ParticipantId::new(principal.as_str()), facts));
            listed.insert(principal, identity);
        }
        self.directory.identities = listed;
        events
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::safety_number;

    #[test]
    fn the_daemon_digit_groups_are_one_number() {
        let row = json!({ "safety_number": ["12345", "67890", "13579"] });
        assert_eq!(safety_number(&row).as_deref(), Some("12345 67890 13579"));
        assert_eq!(
            safety_number(&json!({ "safety_number": "12345 67890" })).as_deref(),
            Some("12345 67890")
        );
        assert_eq!(safety_number(&json!({ "safety_number": [] })), None);
        assert_eq!(safety_number(&json!({ "safety_number": null })), None);
    }
}
