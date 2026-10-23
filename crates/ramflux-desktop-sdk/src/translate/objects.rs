// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The object directory: metadata of the objects stored in this account.
//!
//! Rows are scalar metadata only (no payload, key or token). The daemon
//! lists an object once its bytes are stored here, so a listed object is
//! available unless a transfer or durable upload says otherwise:
//!
//! | row | transfer state |
//! |---|---|
//! | `transfer.state` `running`, `transfer_state` `pending` / `local_committed` | transferring (bytes moved are not reported) |
//! | `transfer.state` `paused` / `failed`, `transfer_state` `failed` | failed, with the daemon's state as reason |
//! | `transfer_state` `available` / `committed`, or `transfer.state` `complete` | complete |
//! | anything else | queued |
//!
//! `direction` `incoming` / `outgoing` is inbound / outbound. An object whose
//! direction the daemon has not observed (an older record) is not shown: the
//! desktop could not tell whether it was sent or received. A null name or
//! size stays unknown.

use ramflux_desktop_core::{Direction, DomainEvent, Object, ObjectId, TransferState};
use serde_json::Value;

use super::Translator;
use crate::directory::field;

fn direction(row: &Value) -> Option<Direction> {
    match row["direction"].as_str() {
        Some("incoming") => Some(Direction::Inbound),
        Some("outgoing") => Some(Direction::Outbound),
        _ => None,
    }
}

fn transfer(row: &Value) -> TransferState {
    let transfer = row["transfer"]["state"].as_str();
    let stored = row["transfer_state"].as_str();
    match (transfer, stored) {
        (Some(state @ ("paused" | "failed")), _) | (_, Some(state @ "failed")) => {
            TransferState::Failed { reason: state.to_owned() }
        }
        (Some("running"), _) | (_, Some("pending" | "local_committed")) => {
            TransferState::Transferring { transferred: 0 }
        }
        (Some("complete"), _) | (_, Some("available" | "committed")) => TransferState::Complete,
        _ => TransferState::Queued,
    }
}

impl Translator {
    /// A complete `object.directory`.
    pub(super) fn object_listing(&mut self, rows: &[Value]) -> Vec<DomainEvent> {
        let mut events = Vec::new();
        let mut directions = std::collections::BTreeMap::new();
        for row in rows {
            let (Some(id), Some(direction)) = (field(row, "id"), direction(row)) else { continue };
            let id = ObjectId::new(id);
            directions.insert(id.clone(), direction);
            let object = Object {
                id: id.clone(),
                name: field(row, "name"),
                size: row["size"].as_u64(),
                direction,
                transfer: transfer(row),
                saved_to: self.objects.get(&id).and_then(|o| o.saved_to.clone()),
                preview: None,
            };
            events.extend(self.put_object(object));
        }
        self.directory.objects = directions;
        events
    }

    /// Emits the object when it differs from what was reported.
    pub(super) fn put_object(&mut self, object: Object) -> Vec<DomainEvent> {
        if self.objects.get(&object.id) == Some(&object) {
            return Vec::new();
        }
        self.objects.insert(object.id.clone(), object.clone());
        vec![DomainEvent::ObjectUpserted(object)]
    }

    /// Records where an object was saved.
    pub(super) fn object_saved(&mut self, id: &ObjectId, path: &str) -> Vec<DomainEvent> {
        let Some(mut object) = self.objects.get(id).cloned() else { return Vec::new() };
        object.saved_to = Some(path.to_owned());
        self.put_object(object)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn transfer_states_map_without_inventing_progress() {
        assert_eq!(transfer(&json!({ "transfer_state": "available" })), TransferState::Complete);
        assert_eq!(
            transfer(&json!({ "transfer_state": "available", "transfer": { "state": "paused" } })),
            TransferState::Failed { reason: "paused".to_owned() }
        );
        assert_eq!(
            transfer(&json!({ "transfer": { "state": "running" } })),
            TransferState::Transferring { transferred: 0 }
        );
        assert_eq!(transfer(&json!({ "transfer_state": "odd" })), TransferState::Queued);
        assert_eq!(direction(&json!({ "direction": null })), None);
    }
}
