// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Contact requests both ways, one per peer and direction.
//!
//! A listing replaces the pending incoming requests (the daemon lists the
//! pending ones; one it no longer lists was answered or expired); an update
//! replaces the one request it names. A request that is no longer pending
//! stays, so its end can be told.
//!
//! Request events carry counts and identifiers only (CONTACT-02 r2): an
//! update of the same request without the private fields (note, preview,
//! safety number, expiry) keeps the ones the last listing gave, until the
//! listing the event calls for replaces them. The private fields are
//! dropped when the account locks.

use super::Store;
use super::changes::Changes;
use crate::model::{ContactRequest, RequestDirection};

impl Store {
    pub(super) fn list_contact_requests(
        &mut self,
        requests: &[ContactRequest],
        changes: &mut Changes,
    ) {
        let before = self.contact_requests.clone();
        self.contact_requests.retain(|(_, direction), request| {
            *direction != RequestDirection::Incoming || !request.state.is_pending()
        });
        for request in requests.iter().filter(|r| r.direction == RequestDirection::Incoming) {
            self.contact_requests
                .insert((request.peer.clone(), request.direction), request.clone());
        }
        if self.contact_requests != before {
            changes.contact_requests = true;
        }
    }

    pub(super) fn upsert_contact_request(
        &mut self,
        request: &ContactRequest,
        changes: &mut Changes,
    ) {
        let key = (request.peer.clone(), request.direction);
        let mut request = request.clone();
        if let Some(known) = self.contact_requests.get(&key).filter(|k| k.id == request.id) {
            request.note = request.note.or_else(|| known.note.clone());
            request.safety_number = request.safety_number.or_else(|| known.safety_number.clone());
            request.expires_at = request.expires_at.or(known.expires_at);
            // The preview is of the first held message: it stays while
            // messages are still held.
            if request.held_messages > 0 {
                request.preview = request.preview.or_else(|| known.preview.clone());
            }
        }
        if self.contact_requests.get(&key) != Some(&request) {
            self.contact_requests.insert(key, request);
            changes.contact_requests = true;
        }
    }

    /// Drops what the requesters wrote (note, preview) and the safety
    /// numbers: private content never outlives an unlocked account.
    pub(super) fn drop_request_content(&mut self, changes: &mut Changes) {
        for request in self.contact_requests.values_mut() {
            if request.note.is_some()
                || request.preview.is_some()
                || request.safety_number.is_some()
            {
                request.note = None;
                request.preview = None;
                request.safety_number = None;
                changes.contact_requests = true;
            }
        }
    }
}
