// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Reads of one desktop's store the steps assert on.

use ramflux_desktop_core::{
    Account, ContextId, LockState, Moment, MomentKind, ParticipantId, Receipt, Store,
};

/// The account of this desktop (each rehearsal desktop has one).
pub fn account(store: &Store) -> Option<&Account> {
    store.accounts().next()
}

/// Whether the account is unlocked.
pub fn unlocked(store: &Store) -> bool {
    account(store).is_some_and(|a| a.lock_state == LockState::Unlocked)
}

/// The message in `context` whose text is `text`.
pub fn message<'s>(store: &'s Store, context: &ContextId, text: &str) -> Option<&'s Moment> {
    store
        .context_moments(context)
        .find(|m| matches!(m.kind, MomentKind::Message(_)) && store.body(&m.id) == Some(text))
}

/// The receipt of the message with `text`, when this account sent it.
pub fn receipt(store: &Store, context: &ContextId, text: &str, me: &str) -> Option<Receipt> {
    let moment = message(store, context, text)?;
    if moment.origin != ParticipantId::new(me) {
        return None;
    }
    match &moment.kind {
        MomentKind::Message(payload) => Some(payload.receipt),
        _ => None,
    }
}

/// Whether a receipt says the recipient's device has the message.
pub fn delivered(receipt: Option<Receipt>) -> bool {
    matches!(receipt, Some(Receipt::Delivered | Receipt::Read))
}

/// The last failures the store kept, for a timeout's message.
pub fn failures(store: &Store) -> String {
    let failures: Vec<String> = store
        .failures()
        .iter()
        .rev()
        .take(3)
        .map(|f| format!("{} failed: {}", f.method, f.message))
        .collect();
    if failures.is_empty() { String::new() } else { format!(" ({})", failures.join("; ")) }
}
