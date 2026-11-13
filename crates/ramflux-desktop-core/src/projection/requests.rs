// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Current-state validation for explicit batches of contact-request decisions.
//! Reply/report await their own backend contracts and never imply acceptance.

use crate::{
    Effect, Intent, IntentError, IntentKind, RequestAnswer, Store, Timestamp, handle_intent,
};
use std::collections::BTreeSet;

/// Maximum selected requests in a single user action.
pub const BATCH_LIMIT: usize = 100;

/// The batch is invalid before any effect is submitted.
#[derive(Debug, thiserror::Error)]
pub enum BatchError {
    /// Empty or oversized selection.
    #[error("select between one and 100 requests")]
    Size,
    /// A repeated ID could cause multiple decisions for one request.
    #[error("duplicate request in selection")]
    Duplicate,
    /// An incoming request expired while its row was displayed.
    #[error("contact request expired")]
    Expired,
    /// Existing contact policy refused one of the decisions.
    #[error(transparent)]
    Policy(#[from] IntentError),
}

/// Validates the complete selection against the same snapshot before returning
/// any effects. Each high-risk block retains its own pending confirmation.
/// Submission is sequential; this does not promise backend batch atomicity.
///
/// # Errors
/// Returns a size, duplicate, expiration or core-policy refusal; no partial effects escape.
pub fn prepare_batch(
    store: &Store,
    ids: &[String],
    answer: RequestAnswer,
    now: Timestamp,
) -> Result<Vec<Effect>, BatchError> {
    if ids.is_empty() || ids.len() > BATCH_LIMIT {
        return Err(BatchError::Size);
    }
    let mut seen = BTreeSet::new();
    let mut effects = Vec::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(BatchError::Duplicate);
        }
        if store.contact_request_by_id(id).is_some_and(|r| r.expires_at.is_some_and(|at| at <= now))
        {
            return Err(BatchError::Expired);
        }
        effects.extend(handle_intent(
            store,
            Intent::human(IntentKind::AnswerContactRequest { request_id: id.clone(), answer }),
            now,
        )?);
    }
    Ok(effects)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ContactRequest, DomainEvent, EventEnvelope, ParticipantId, RequestDirection, RequestState,
    };

    fn store() -> Store {
        let mut store = Store::new();
        let request = ContactRequest {
            id: "r".into(),
            peer: ParticipantId::new("peer"),
            direction: RequestDirection::Incoming,
            state: RequestState::Pending,
            note: None,
            safety_number: None,
            held_messages: 2,
            preview: None,
            expires_at: Some(Timestamp(100)),
        };
        store
            .apply(&EventEnvelope::new(
                1,
                Timestamp(0),
                DomainEvent::ContactRequestUpserted(request),
            ))
            .expect("test: pending source request");
        store
    }

    #[test]
    fn batches_validate_all_rows_before_returning_effects_and_preserve_holds() {
        let store = store();
        assert!(
            matches!(
                prepare_batch(
                    &store,
                    &["r".into(), "r".into()],
                    RequestAnswer::Decline,
                    Timestamp(1)
                ),
                Err(BatchError::Duplicate)
            ),
            "a repeated request cannot produce a repeated decision"
        );
        assert!(
            matches!(
                prepare_batch(
                    &store,
                    &["r".into(), "missing".into()],
                    RequestAnswer::Decline,
                    Timestamp(1)
                ),
                Err(BatchError::Policy(_))
            ),
            "a stale selection cannot leak partial effects"
        );
        assert!(
            matches!(
                prepare_batch(&store, &["r".into()], RequestAnswer::Decline, Timestamp(100)),
                Err(BatchError::Expired)
            ),
            "expiration is inclusive"
        );
        let effects = prepare_batch(&store, &["r".into()], RequestAnswer::Block, Timestamp(1))
            .expect("test: valid block selection");
        assert!(
            matches!(effects.as_slice(), [Effect::RequireConfirmation { .. }]),
            "batching cannot bypass attended confirmation"
        );
        assert!(
            store.contact_request_by_id("r").is_some_and(|r| r.state == RequestState::Pending),
            "planning cannot decide the request in the store"
        );
    }
}
