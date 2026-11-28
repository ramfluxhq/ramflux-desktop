// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Retry keys for mutating requests.
//!
//! The daemon binds a frame's `operation_id` (1 to 256 bytes) to the
//! request's method and body in a durable receipt: a retry with the same key
//! and the same intent returns the stored answer without applying it again,
//! a different intent under the same key is refused. Lifecycle retries join
//! the live execution; unlock, lock and switch can also recover an abandoned
//! reservation. Other indeterminate outcomes return the typed
//! `OperationReconciliationRequired` refusal.
//!
//! Each effect the desktop performs gets one fresh key when it is planned.
//! The key is reused only when the bridge resends that same request after
//! the answer was lost (a broken connection or a missed deadline), never for
//! a new effect: a user who blocks, unblocks and blocks again performs three
//! operations, not one.
//!
//! Reads, pulls (`message.receive`, `group.receive`) and probes carry no key:
//! they are safe to repeat, and a key per poll would leave one receipt per
//! poll in the account database.

/// Methods that are planned as effects but do not change daemon state.
/// `object.get` answers with the object's bytes, which must never be kept
/// in a receipt; `contact.card.export` only reads this account's card.
const NOT_MUTATIONS: [&str; 6] = [
    "node.probe",
    "object.get",
    "contact.card.export",
    "username.resolve",
    "contact.verification.status",
    "conversation.security",
];

/// The methods the daemon's agent module answers. It refuses any frame of
/// these that carries an `operation_id` (`CustodySetupInvalid`): their
/// retries are keyed inside the body instead (a creation's `request_id`).
/// The `rf` CLI sends them without a frame key either.
const AGENT_UNKEYED: [&str; 14] = [
    "agent.create",
    "agent.creation.decide",
    "agent.operation.get",
    "agent.list",
    "agent.get",
    "agent.watch",
    "agent.delete",
    "agent.retire",
    "agent.persona.edit",
    "agent.access.set",
    "agent.catalog",
    "agent.update",
    "agent.pause",
    "agent.resume",
];

/// Prefix of desktop keys (readable in daemon traces).
const KEY_PREFIX: &str = "desk-op-";

/// Whether an effect request with `method` changes daemon state.
pub fn is_mutation(method: &str) -> bool {
    !NOT_MUTATIONS.contains(&method) && !AGENT_UNKEYED.contains(&method)
}

/// A fresh retry key.
///
/// # Errors
///
/// Returns a message when the platform cannot supply randomness.
pub fn fresh_key() -> Result<String, String> {
    let random = zeroize::Zeroizing::new(
        ramflux_crypto::random_32().map_err(|error| format!("no randomness: {error}"))?,
    );
    let hex: String = random.iter().take(16).map(|b| format!("{b:02x}")).collect();
    Ok(format!("{KEY_PREFIX}{hex}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_fresh_and_within_the_daemon_limit() {
        let first = fresh_key().expect("randomness");
        let second = fresh_key().expect("randomness");
        assert_ne!(first, second);
        assert!(first.starts_with(KEY_PREFIX));
        assert!(!first.is_empty() && first.len() <= 256);
        assert!(is_mutation("contact.block"));
        assert!(!is_mutation("node.probe"));
        assert!(!is_mutation("object.get"));
        // The daemon refuses a frame key on these.
        assert!(!is_mutation("agent.create"));
        assert!(!is_mutation("agent.delete"));
        assert!(!is_mutation("agent.persona.edit"));
        assert!(!is_mutation("agent.access.set"));
    }
}
