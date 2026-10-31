// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The daemon side of the born-v3 creation ceremony, for a mock source.
//!
//! It follows the contract the desktop is built against (contract-first:
//! the backend has the same five methods): begin prepares the recovery
//! material and a five-minute deadline, show-once hands the phrase out
//! once, hidden-challenge chooses the three positions to ask, confirm is
//! the single attempt, cancel erases everything. A step out of order, or
//! after the deadline, ends the ceremony and erases the phrase.
//!
//! The phrase is not random here: tests need to know the words. It is a
//! fixed non-production sequence of list words, never a real phrase.
//!
//! A scenario rule for a ceremony method wins over this engine (a fixture
//! may script a daemon that refuses to begin).

use ramflux_desktop_core::{
    Account, AccountId, AccountKind, CEREMONY_TTL_MS, Ceremony, CeremonyAccount, CeremonyFailure,
    CeremonyPhase, DomainEvent, Effect, EffectFailure, FailureCode, LockState, Networking,
    RecoveryPhrase, SecureSessions, Timestamp, WordInput, methods, recovery::words, resolve_word,
};
use zeroize::Zeroizing;

/// The positions the mock asks for by default (one-based, all different).
pub const DEMO_POSITIONS: [u8; 3] = [3, 11, 20];

/// The word at one-based `position` of the mock's phrase: a fixed
/// non-production sequence of 24 different list words.
pub fn demo_word(position: usize) -> &'static str {
    let list = words::wordlist();
    list[(position * 83 + 7) % list.len()]
}

fn demo_phrase() -> Zeroizing<String> {
    Zeroizing::new((1..=24).map(demo_word).collect::<Vec<_>>().join(" "))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Idle,
    Prepared,
    Shown,
    Challenged,
}

/// The mock daemon's ceremony.
#[derive(Debug)]
pub struct MockCeremony {
    step: Step,
    positions: [u8; 3],
    ceremony_id: String,
    label: String,
    expires_at: Timestamp,
    deadline_ms: u64,
    began: u32,
    /// The phrase handed out and not taken yet.
    parked: Option<RecoveryPhrase>,
    /// What the created account reports for its networking.
    networking: Networking,
    /// The passphrase each created account was given, by account id.
    passphrases: std::collections::BTreeMap<String, Zeroizing<String>>,
    /// Methods whose next call fails as a failed effect (a broken
    /// connection, a timeout): no ceremony update comes, the ceremony is
    /// left as it was.
    failing: Vec<&'static str>,
    /// How the next begin is refused, as a ceremony update that ended
    /// nothing (the vault is missing, the daemon is not offering).
    refusal: Option<CeremonyFailure>,
    /// The recovery of an account that lives on this device.
    recovery: crate::mock_recovery::MockRecovery,
    /// The custodian of the AI participant being created, when it is one.
    custodian: Option<String>,
    /// The credential exports of AI participants.
    agents: crate::mock_agents::MockAgents,
}

impl Default for MockCeremony {
    fn default() -> Self {
        Self::new(DEMO_POSITIONS)
    }
}

impl MockCeremony {
    /// A ceremony that asks for `positions`.
    pub fn new(positions: [u8; 3]) -> Self {
        Self {
            step: Step::Idle,
            positions,
            ceremony_id: String::new(),
            label: String::new(),
            expires_at: Timestamp(0),
            deadline_ms: 0,
            began: 0,
            parked: None,
            networking: Networking::Pending,
            passphrases: std::collections::BTreeMap::new(),
            failing: Vec::new(),
            refusal: None,
            recovery: crate::mock_recovery::MockRecovery::default(),
            custodian: None,
            agents: crate::mock_agents::MockAgents::default(),
        }
    }

    /// The mock daemon's credential exports of AI participants.
    pub fn agents_mut(&mut self) -> &mut crate::mock_agents::MockAgents {
        &mut self.agents
    }

    /// The mock daemon's recovery of accounts that live on this device.
    pub fn recovery_mut(&mut self) -> &mut crate::mock_recovery::MockRecovery {
        &mut self.recovery
    }

    /// The next call of `method` fails as a failed effect, with no ceremony
    /// update (the ceremony, if any, stays as it was).
    pub fn fail_next(&mut self, method: &'static str) {
        self.failing.push(method);
    }

    /// The next begin is refused with `failure`, as a ceremony update that
    /// ended nothing.
    pub fn refuse_next_begin(&mut self, failure: CeremonyFailure) {
        self.refusal = Some(failure);
    }

    /// Chooses the positions asked from now on.
    pub fn set_positions(&mut self, positions: [u8; 3]) {
        self.positions = positions;
    }

    /// Chooses what the created account reports for its networking.
    pub fn set_networking(&mut self, networking: Networking) {
        self.networking = networking;
    }

    /// The phrase handed out and not taken yet (once).
    pub fn take_phrase(&mut self) -> Option<RecoveryPhrase> {
        self.parked.take()
    }

    /// Whether the mock still holds a phrase (handed out and not taken, or
    /// not handed out yet).
    pub fn holds_phrase(&self) -> bool {
        self.parked.is_some() || matches!(self.step, Step::Prepared | Step::Shown)
    }

    /// Answers a ceremony method; `None` for any other effect. `now_ms` is
    /// the source's time since its epoch, `epoch` its origin.
    pub fn answer(
        &mut self,
        effect: &Effect,
        now_ms: u64,
        epoch: Timestamp,
    ) -> Option<Vec<DomainEvent>> {
        let method = effect.method()?;
        if let Some(events) = self.unlock(effect) {
            return Some(events);
        }
        if let Some(index) = self.failing.iter().position(|failing| *failing == method) {
            self.failing.remove(index);
            return Some(vec![DomainEvent::EffectFailed(EffectFailure {
                method: method.to_owned(),
                code: FailureCode::Transport,
                message: "local bus connection lost".to_owned(),
            })]);
        }
        if let Some(events) = self.recovery.answer(effect, now_ms, epoch) {
            return Some(events);
        }
        if let Some(events) = self.agents.answer(effect) {
            return Some(events);
        }
        let ours = [
            methods::CUSTODY_SHOW_ONCE,
            methods::CUSTODY_HIDDEN_CHALLENGE,
            methods::CUSTODY_CONFIRM,
            methods::CUSTODY_CANCEL,
        ];
        // Begin is `account.create` without a secret; the one-shot create
        // (which carries the passphrase) is not ours.
        let begin = method == methods::CUSTODY_BEGIN && matches!(effect, Effect::Call { .. });
        if !begin && !ours.contains(&method) {
            return None;
        }
        if method == methods::CUSTODY_BEGIN {
            if let Some(failure) = self.refusal.take() {
                self.label = String::new();
                return Some(self.end(CeremonyPhase::Failed(failure)));
            }
            return Some(self.begin(effect, now_ms, epoch));
        }
        if method == methods::CUSTODY_CANCEL {
            return Some(self.cancel());
        }
        if self.step != Step::Idle && now_ms >= self.deadline_ms {
            return Some(self.end(CeremonyPhase::Failed(CeremonyFailure::Expired)));
        }
        Some(match (method, self.step) {
            (methods::CUSTODY_SHOW_ONCE, Step::Prepared) => {
                self.parked = RecoveryPhrase::parse(demo_phrase()).ok();
                self.step = Step::Shown;
                self.record(CeremonyPhase::PhraseReady)
            }
            (methods::CUSTODY_HIDDEN_CHALLENGE, Step::Shown) => {
                // A phrase the screen never took is gone with the hide.
                self.parked = None;
                self.step = Step::Challenged;
                self.record(CeremonyPhase::Challenged { positions: self.positions })
            }
            (methods::CUSTODY_CONFIRM, Step::Challenged) => self.confirm(effect),
            _ => self.end(CeremonyPhase::Failed(CeremonyFailure::Invalid)),
        })
    }

    fn begin(&mut self, effect: &Effect, now_ms: u64, epoch: Timestamp) -> Vec<DomainEvent> {
        let label = effect
            .body()
            .and_then(|body| body.get("display_name"))
            .and_then(ramflux_desktop_core::JsonValue::as_str)
            .unwrap_or_default()
            .to_owned();
        let custodian = effect
            .body()
            .filter(|body| {
                body.get("principal_kind").and_then(|k| k.as_str()) == Some("headless_ai")
            })
            .map(|body| {
                body.get("custodian_account")
                    .and_then(ramflux_desktop_core::JsonValue::as_str)
                    .unwrap_or_default()
                    .to_owned()
            });
        if self.step != Step::Idle || label.is_empty() {
            // One ceremony at a time; the running one is erased.
            return self.end(CeremonyPhase::Failed(CeremonyFailure::Invalid));
        }
        // An AI participant needs a custodian the daemon accepts.
        if custodian.as_deref().is_some_and(str::is_empty) {
            return self.end(CeremonyPhase::Failed(CeremonyFailure::CustodianRefused));
        }
        self.custodian = custodian;
        self.began += 1;
        self.step = Step::Prepared;
        self.ceremony_id = format!("ceremony-mock-{}", self.began);
        self.label = label;
        self.deadline_ms = now_ms.saturating_add(CEREMONY_TTL_MS.unsigned_abs());
        self.expires_at = epoch.saturating_add_ms(self.deadline_ms);
        self.record(CeremonyPhase::Prepared)
    }

    fn cancel(&mut self) -> Vec<DomainEvent> {
        if self.step == Step::Idle {
            return Vec::new();
        }
        self.end(CeremonyPhase::Cancelled)
    }

    fn confirm(&mut self, effect: &Effect) -> Vec<DomainEvent> {
        let Effect::SecretCall { secrets, .. } = effect else {
            return self.end(CeremonyPhase::Failed(CeremonyFailure::Invalid));
        };
        // One attempt, whatever the outcome: the ceremony is consumed.
        // The passphrase travels with the words (`{words, passphrase}`): a
        // confirm without one is refused as not valid.
        // An AI participant's confirm carries the three words and no
        // passphrase (the daemon makes its own); an account's carries both.
        let agent = self.custodian.clone();
        let ([first, second, third], passphrase) = match (secrets.as_slice(), &agent) {
            ([first, second, third], Some(_)) => ([first, second, third], None),
            ([first, second, third, passphrase], None) if !passphrase.is_empty() => {
                ([first, second, third], Some(passphrase))
            }
            _ => return self.end(CeremonyPhase::Failed(CeremonyFailure::Invalid)),
        };
        let matches = self.positions.iter().zip([first, second, third]).all(|(position, typed)| {
            matches!(
                resolve_word(typed.expose()),
                WordInput::Word(word) | WordInput::Prefix(word)
                    if word == demo_word(usize::from(*position))
            )
        });
        if !matches {
            return self.end(CeremonyPhase::Failed(CeremonyFailure::ConfirmationFailed));
        }
        let id = format!("acct-{}-{}", if agent.is_some() { "agent" } else { "v3" }, self.began);
        if let Some(passphrase) = passphrase {
            self.passphrases.insert(id.clone(), Zeroizing::new(passphrase.expose().to_owned()));
        }
        let account = CeremonyAccount {
            local_account_id: id.clone(),
            display_name: self.label.clone(),
            networking: self.networking,
        };
        // The ordinary account-create answer: the account is listed,
        // unlocked, with its networking state.
        let listed = DomainEvent::AccountUpdated(Account {
            id: AccountId::new(id),
            label: self.label.clone(),
            kind: if agent.is_some() {
                AccountKind::HeadlessAi
            } else {
                AccountKind::HumanInteractive
            },
            node_id: None,
            device_id: Some("dev-mock".to_owned()),
            lock_state: LockState::Unlocked,
            self_participant_id: None,
            secure_sessions: SecureSessions::Ready,
            networking: self.networking,
        });
        let mut events = vec![listed];
        if let Some(custodian) = agent {
            events.push(DomainEvent::CustodianKnown {
                agent: AccountId::new(&account.local_account_id),
                custodian: AccountId::new(custodian),
            });
        }
        events.extend(self.end(CeremonyPhase::Created(account)));
        events
    }

    /// Unlocking an account this ceremony made: the passphrase set at
    /// confirm opens it; any other is refused.
    fn unlock(&self, effect: &Effect) -> Option<Vec<DomainEvent>> {
        let Effect::SecretCall { method, body, secrets } = effect else { return None };
        if method != methods::ACCOUNT_UNLOCK {
            return None;
        }
        let id = body.get("account_id").and_then(ramflux_desktop_core::JsonValue::as_str)?;
        let expected = self.passphrases.get(id)?;
        let right = secrets.first().is_some_and(|typed| typed.expose() == expected.as_str());
        Some(vec![if right {
            DomainEvent::AccountLockChanged {
                account_id: AccountId::new(id),
                lock_state: LockState::Unlocked,
            }
        } else {
            DomainEvent::EffectFailed(EffectFailure {
                method: method.clone(),
                code: FailureCode::Daemon("wrong_passphrase".to_owned()),
                message: "local bus error: wrong passphrase".to_owned(),
            })
        }])
    }

    /// Ends the ceremony with `phase`, erasing the phrase.
    fn end(&mut self, phase: CeremonyPhase) -> Vec<DomainEvent> {
        self.parked = None;
        self.custodian = None;
        let refused_begin = self.step == Step::Idle;
        self.step = Step::Idle;
        let mut events = self.record(phase);
        if refused_begin && let Some(DomainEvent::CeremonyUpdated(ceremony)) = events.first_mut() {
            ceremony.ceremony_id = String::new();
            ceremony.expires_at = Timestamp(0);
        }
        events
    }

    fn record(&self, phase: CeremonyPhase) -> Vec<DomainEvent> {
        vec![DomainEvent::CeremonyUpdated(Ceremony {
            phase,
            ceremony_id: self.ceremony_id.clone(),
            label: self.label.clone(),
            expires_at: self.expires_at,
        })]
    }
}

#[cfg(test)]
mod tests {
    use ramflux_desktop_core::{JsonValue, SecretInput};
    use serde_json::json;

    use super::*;

    const EPOCH: Timestamp = Timestamp(1_000_000);

    fn call(method: &str, body: JsonValue) -> Effect {
        Effect::Call { method: method.to_owned(), body }
    }

    fn confirm(words: [&str; 3]) -> Effect {
        Effect::SecretCall {
            method: methods::CUSTODY_CONFIRM.to_owned(),
            body: json!({}),
            secrets: words
                .iter()
                .map(|w| SecretInput::new(*w))
                .chain([SecretInput::new("river stone lantern")])
                .collect(),
        }
    }

    fn phase(events: &[DomainEvent]) -> CeremonyPhase {
        let ceremony = events.iter().find_map(|event| match event {
            DomainEvent::CeremonyUpdated(ceremony) => Some(ceremony),
            _ => None,
        });
        ceremony.expect("a ceremony update").phase.clone()
    }

    fn begun(mock: &mut MockCeremony) {
        let begin = call(methods::CUSTODY_BEGIN, json!({ "display_name": "Alex" }));
        assert_eq!(phase(&mock.answer(&begin, 0, EPOCH).expect("ours")), CeremonyPhase::Prepared);
    }

    fn asked(mock: &mut MockCeremony) {
        begun(mock);
        mock.answer(&call(methods::CUSTODY_SHOW_ONCE, json!({})), 1, EPOCH);
        mock.answer(&call(methods::CUSTODY_HIDDEN_CHALLENGE, json!({})), 2, EPOCH);
    }

    #[test]
    fn the_demo_phrase_has_24_different_list_words() {
        let phrase = RecoveryPhrase::parse(demo_phrase()).expect("phrase");
        let distinct: std::collections::BTreeSet<&str> = phrase.words().collect();
        assert_eq!(distinct.len(), 24);
        assert_eq!(phrase.word(3), Some(demo_word(3)));
    }

    #[test]
    fn the_one_shot_create_with_a_passphrase_is_not_ours() {
        let mut mock = MockCeremony::default();
        let legacy = Effect::SecretCall {
            method: methods::ACCOUNT_CREATE.to_owned(),
            body: json!({ "label": "Alex" }),
            secrets: vec![SecretInput::new("river stone lantern")],
        };
        assert!(mock.answer(&legacy, 0, EPOCH).is_none());
    }

    #[test]
    fn other_effects_are_not_ours() {
        let mut mock = MockCeremony::default();
        assert!(mock.answer(&call(methods::SEND_MESSAGE, json!({})), 0, EPOCH).is_none());
    }

    #[test]
    fn the_phrase_is_handed_out_once() {
        let mut mock = MockCeremony::default();
        begun(&mut mock);
        let show = call(methods::CUSTODY_SHOW_ONCE, json!({}));
        assert_eq!(phase(&mock.answer(&show, 1, EPOCH).expect("ours")), CeremonyPhase::PhraseReady);
        assert!(mock.take_phrase().is_some());
        assert!(mock.take_phrase().is_none(), "taken once");
        // Showing again is a replay: refused, and the ceremony is erased.
        assert_eq!(
            phase(&mock.answer(&show, 2, EPOCH).expect("ours")),
            CeremonyPhase::Failed(CeremonyFailure::Invalid)
        );
        assert!(!mock.holds_phrase());
    }

    #[test]
    fn the_three_right_words_create_the_account_and_a_prefix_will_do() {
        let mut mock = MockCeremony::default();
        asked(&mut mock);
        let [a, b, c] = DEMO_POSITIONS.map(|p| demo_word(usize::from(p)));
        let events = mock.answer(&confirm([a, &b[..b.len().min(4)], c]), 3, EPOCH).expect("ours");
        let CeremonyPhase::Created(account) = phase(&events) else { panic!("account") };
        assert_eq!(account.display_name, "Alex");
    }

    #[test]
    fn a_confirm_without_a_passphrase_is_refused_and_consumes_the_ceremony() {
        let mut mock = MockCeremony::default();
        asked(&mut mock);
        let [a, b, c] = DEMO_POSITIONS.map(|p| demo_word(usize::from(p)));
        let bare = Effect::SecretCall {
            method: methods::CUSTODY_CONFIRM.to_owned(),
            body: json!({}),
            secrets: [a, b, c].iter().map(|w| SecretInput::new(*w)).collect(),
        };
        let events = mock.answer(&bare, 3, EPOCH).expect("ours");
        assert_eq!(phase(&events), CeremonyPhase::Failed(CeremonyFailure::Invalid));
    }

    #[test]
    fn a_wrong_word_spends_the_only_attempt() {
        let mut mock = MockCeremony::default();
        asked(&mut mock);
        let [a, b, _] = DEMO_POSITIONS.map(|p| demo_word(usize::from(p)));
        let wrong = demo_word(5);
        let events = mock.answer(&confirm([a, b, wrong]), 3, EPOCH).expect("ours");
        assert_eq!(phase(&events), CeremonyPhase::Failed(CeremonyFailure::ConfirmationFailed));
        // Nothing is left to try again with.
        let again = mock.answer(&confirm([a, b, demo_word(20)]), 4, EPOCH).expect("ours");
        assert_eq!(phase(&again), CeremonyPhase::Failed(CeremonyFailure::Invalid));
    }

    #[test]
    fn a_step_after_the_deadline_ends_the_ceremony_as_expired() {
        let mut mock = MockCeremony::default();
        begun(&mut mock);
        let late = CEREMONY_TTL_MS.unsigned_abs();
        let show = call(methods::CUSTODY_SHOW_ONCE, json!({}));
        let events = mock.answer(&show, late, EPOCH).expect("ours");
        assert_eq!(phase(&events), CeremonyPhase::Failed(CeremonyFailure::Expired));
        assert!(!mock.holds_phrase());
    }

    #[test]
    fn cancel_erases_the_phrase_the_screen_never_took() {
        let mut mock = MockCeremony::default();
        begun(&mut mock);
        mock.answer(&call(methods::CUSTODY_SHOW_ONCE, json!({})), 1, EPOCH);
        let cancel = call(methods::CUSTODY_CANCEL, json!({}));
        assert_eq!(phase(&mock.answer(&cancel, 2, EPOCH).expect("ours")), CeremonyPhase::Cancelled);
        assert!(mock.take_phrase().is_none());
    }
}
