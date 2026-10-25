// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Contact requests both ways, one per peer and direction.
//!
//! A listing replaces the pending incoming requests (the daemon lists the
//! pending ones; one it no longer lists was answered or expired); an update
//! replaces the one request it names. A request that is no longer pending
//! stays, so its end can be told.

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
        if self.contact_requests.get(&key) != Some(request) {
            self.contact_requests.insert(key, request.clone());
            changes.contact_requests = true;
        }
    }
}
