// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The daemon side of the born-v3 recovery ceremony, for a mock source.
//!
//! It follows the bus contract of the product revision the desktop is pinned
//! to: begin names the account and answers its public fingerprint (the
//! anchor) with a five-minute deadline; confirm carries the phrase, the
//! passphrase of the account on this device and the fingerprint the user
//! confirmed, once; cancel erases. Wrong words or a fingerprint other than
//! the named one end the recovery as a mismatch; a wrong passphrase ends it
//! as not valid; nothing changes in either case.
//!
//! The phrase is the mock creation's fixed non-production sequence
//! ([`crate::mock_ceremony::demo_word`]).

use std::collections::BTreeMap;

use ramflux_desktop_core::{
    Account, AccountId, AccountKind, CEREMONY_TTL_MS, CeremonyAccount, CeremonyFailure,
    DomainEvent, Effect, LockState, Networking, RecoveryCeremony, RecoveryPhase, SecureSessions,
    Timestamp, methods,
};

use crate::mock_ceremony::demo_word;

/// The fingerprint the mock names for every account: 43 base64url
/// characters, the shape of the product's BLAKE3-256 commitment.
pub const DEMO_ANCHOR: &str = "k3J9xQ2mP8vL1nRtYb6WcZ4dHf7GsA0eUiO5TqMaV-E";

/// The fixed phrase of the mock.
pub fn demo_phrase() -> String {
    (1..=24).map(demo_word).collect::<Vec<_>>().join(" ")
}

#[derive(Debug)]
struct Running {
    account_id: String,
    ceremony_id: String,
    deadline_ms: u64,
    expires_at: Timestamp,
}

/// The mock daemon's recovery.
#[derive(Debug, Default)]
pub struct MockRecovery {
    /// The accounts a recovery may name: their label and passphrase.
    accounts: BTreeMap<String, (String, String)>,
    running: Option<Running>,
    began: u32,
    networking: Option<Networking>,
}

impl MockRecovery {
    /// Lets a recovery name `id` (whose passphrase on this device is
    /// `passphrase`).
    pub fn allow(&mut self, id: &str, label: &str, passphrase: &str) {
        self.accounts.insert(id.to_owned(), (label.to_owned(), passphrase.to_owned()));
    }

    /// Chooses what the recovered account reports for its networking.
    pub fn set_networking(&mut self, networking: Networking) {
        self.networking = Some(networking);
    }

    /// Whether a recovery is running.
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Answers a recovery method; `None` for any other effect.
    pub fn answer(
        &mut self,
        effect: &Effect,
        now_ms: u64,
        epoch: Timestamp,
    ) -> Option<Vec<DomainEvent>> {
        let method = effect.method()?;
        Some(match method {
            methods::RECOVER_BEGIN => self.begin(effect, now_ms, epoch),
            methods::RECOVER_CANCEL => match self.running.take() {
                Some(running) => vec![Self::record(&running, "", RecoveryPhase::Cancelled)],
                None => Vec::new(),
            },
            methods::RECOVER_CONFIRM => self.confirm(effect, now_ms),
            _ => return None,
        })
    }

    fn record(running: &Running, anchor: &str, phase: RecoveryPhase) -> DomainEvent {
        DomainEvent::RecoveryUpdated(RecoveryCeremony {
            phase,
            account_id: AccountId::new(&running.account_id),
            anchor: anchor.to_owned(),
            ceremony_id: running.ceremony_id.clone(),
            expires_at: running.expires_at,
        })
    }

    fn refused(account: &str, failure: CeremonyFailure) -> Vec<DomainEvent> {
        vec![DomainEvent::RecoveryUpdated(RecoveryCeremony {
            phase: RecoveryPhase::Failed(failure),
            account_id: AccountId::new(account),
            anchor: String::new(),
            ceremony_id: String::new(),
            expires_at: Timestamp(0),
        })]
    }

    fn begin(&mut self, effect: &Effect, now_ms: u64, epoch: Timestamp) -> Vec<DomainEvent> {
        let id = effect
            .body()
            .and_then(|body| body.get("account_id"))
            .and_then(ramflux_desktop_core::JsonValue::as_str)
            .unwrap_or_default()
            .to_owned();
        // One recovery at a time; the running one is erased.
        self.running = None;
        if !self.accounts.contains_key(&id) {
            return Self::refused(&id, CeremonyFailure::Invalid);
        }
        self.began += 1;
        let running = Running {
            account_id: id,
            ceremony_id: format!("recovery-mock-{}", self.began),
            deadline_ms: now_ms.saturating_add(CEREMONY_TTL_MS.unsigned_abs()),
            expires_at: epoch
                .saturating_add_ms(now_ms.saturating_add(CEREMONY_TTL_MS.unsigned_abs())),
        };
        let event = Self::record(&running, DEMO_ANCHOR, RecoveryPhase::Prepared);
        self.running = Some(running);
        vec![event]
    }

    fn confirm(&mut self, effect: &Effect, now_ms: u64) -> Vec<DomainEvent> {
        let Some(running) = self.running.take() else {
            return Self::refused("", CeremonyFailure::Invalid);
        };
        let end =
            |failure| vec![Self::record(&running, DEMO_ANCHOR, RecoveryPhase::Failed(failure))];
        if now_ms >= running.deadline_ms {
            return end(CeremonyFailure::Expired);
        }
        let Effect::SecretCall { body, secrets, .. } = effect else {
            return end(CeremonyFailure::Invalid);
        };
        let [phrase, passphrase] = secrets.as_slice() else {
            return end(CeremonyFailure::Invalid);
        };
        let anchor = body.get("confirmed_anchor").and_then(ramflux_desktop_core::JsonValue::as_str);
        if anchor != Some(DEMO_ANCHOR) || phrase.expose() != demo_phrase() {
            return end(CeremonyFailure::ConfirmationFailed);
        }
        let Some((label, expected)) = self.accounts.get(&running.account_id) else {
            return end(CeremonyFailure::Invalid);
        };
        if passphrase.expose() != expected {
            return end(CeremonyFailure::Invalid);
        }
        let networking = self.networking.unwrap_or(Networking::Pending);
        let account = CeremonyAccount {
            local_account_id: running.account_id.clone(),
            display_name: label.clone(),
            networking,
        };
        vec![
            DomainEvent::AccountUpdated(Account {
                id: AccountId::new(&running.account_id),
                label: label.clone(),
                kind: AccountKind::HumanInteractive,
                node_id: None,
                device_id: Some("dev-mock".to_owned()),
                lock_state: LockState::Unlocked,
                self_participant_id: None,
                secure_sessions: SecureSessions::Ready,
                networking,
            }),
            Self::record(&running, DEMO_ANCHOR, RecoveryPhase::Recovered(account)),
        ]
    }
}

#[cfg(test)]
mod tests {
    use ramflux_desktop_core::{JsonValue, SecretInput};
    use serde_json::json;

    use super::*;

    const EPOCH: Timestamp = Timestamp(1_000_000);

    fn mock() -> MockRecovery {
        let mut mock = MockRecovery::default();
        mock.allow("a-1", "Alex", "river stone lantern");
        mock
    }

    fn call(method: &str, body: JsonValue) -> Effect {
        Effect::Call { method: method.to_owned(), body }
    }

    fn confirm(phrase: &str, passphrase: &str, anchor: &str) -> Effect {
        Effect::SecretCall {
            method: methods::RECOVER_CONFIRM.to_owned(),
            body: json!({ "account_id": "a-1", "confirmed_anchor": anchor }),
            secrets: vec![SecretInput::new(phrase), SecretInput::new(passphrase)],
        }
    }

    fn phase(events: &[DomainEvent]) -> RecoveryPhase {
        events
            .iter()
            .find_map(|event| match event {
                DomainEvent::RecoveryUpdated(record) => Some(record.phase.clone()),
                _ => None,
            })
            .expect("a recovery update")
    }

    fn begun(mock: &mut MockRecovery) {
        let begin = call(methods::RECOVER_BEGIN, json!({ "account_id": "a-1" }));
        let events = mock.answer(&begin, 0, EPOCH).expect("ours");
        assert_eq!(phase(&events), RecoveryPhase::Prepared);
    }

    #[test]
    fn an_unknown_account_is_refused_with_no_recovery_behind_it() {
        let mut mock = mock();
        let begin = call(methods::RECOVER_BEGIN, json!({ "account_id": "nobody" }));
        let events = mock.answer(&begin, 0, EPOCH).expect("ours");
        assert_eq!(phase(&events), RecoveryPhase::Failed(CeremonyFailure::Invalid));
        assert!(!mock.is_running());
    }

    #[test]
    fn the_right_phrase_passphrase_and_fingerprint_recover_the_account() {
        let mut mock = mock();
        begun(&mut mock);
        let events = mock
            .answer(&confirm(&demo_phrase(), "river stone lantern", DEMO_ANCHOR), 5, EPOCH)
            .expect("ours");
        assert!(matches!(phase(&events), RecoveryPhase::Recovered(_)));
        assert!(matches!(events[0], DomainEvent::AccountUpdated(_)), "the account comes first");
        assert!(!mock.is_running(), "the one attempt is consumed");
    }

    #[test]
    fn wrong_words_another_fingerprint_or_a_wrong_passphrase_end_it_without_a_change() {
        for (phrase, passphrase, anchor, expected) in [
            (
                "abandon ability",
                "river stone lantern",
                DEMO_ANCHOR,
                CeremonyFailure::ConfirmationFailed,
            ),
            (
                demo_phrase().as_str(),
                "river stone lantern",
                "other",
                CeremonyFailure::ConfirmationFailed,
            ),
            (demo_phrase().as_str(), "nope", DEMO_ANCHOR, CeremonyFailure::Invalid),
        ] {
            let mut mock = mock();
            begun(&mut mock);
            let events = mock.answer(&confirm(phrase, passphrase, anchor), 5, EPOCH).expect("ours");
            assert_eq!(phase(&events), RecoveryPhase::Failed(expected));
            assert!(!events.iter().any(|e| matches!(e, DomainEvent::AccountUpdated(_))));
            assert!(!mock.is_running());
        }
    }

    #[test]
    fn a_confirm_after_the_deadline_is_expired_and_cancel_erases() {
        let mut mock = mock();
        begun(&mut mock);
        let late = CEREMONY_TTL_MS.unsigned_abs();
        let events = mock
            .answer(&confirm(&demo_phrase(), "river stone lantern", DEMO_ANCHOR), late, EPOCH)
            .expect("ours");
        assert_eq!(phase(&events), RecoveryPhase::Failed(CeremonyFailure::Expired));
        begun(&mut mock);
        let cancel = call(methods::RECOVER_CANCEL, json!({}));
        assert_eq!(phase(&mock.answer(&cancel, 1, EPOCH).expect("ours")), RecoveryPhase::Cancelled);
        assert!(!mock.is_running());
    }

    #[test]
    fn other_effects_are_not_ours() {
        let mut mock = mock();
        assert!(mock.answer(&call(methods::SEND_MESSAGE, json!({})), 0, EPOCH).is_none());
    }
}
