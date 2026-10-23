// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! One account's store and source, with a scenario clock.
//!
//! This is the only code in the application that calls [`Store::apply`] and
//! [`Source::submit`] (rules R1 and R2). The GPUI [`super::StoreHandle`] owns
//! one session per account and drives it; views never reach a session.

use ramflux_desktop_core::{
    Effect, EventEnvelope, Intent, IntentError, Source, Store, Timestamp, handle_intent,
};
use ramflux_desktop_test::{MockSource, Scenario};

use super::changes::ChangeSet;
use super::inspector::{InspectorEntry, effect_label, event_name, intent_name};

/// What one clock movement or step applied.
#[derive(Debug, Default)]
pub struct Applied {
    /// Merged changes.
    pub changes: ChangeSet,
    /// Inspector records for the applied envelopes.
    pub records: Vec<InspectorEntry>,
}

/// The policy result of a dispatched intent.
#[derive(Debug)]
pub enum DispatchOutcome {
    /// Effects went to the source; labels for display.
    Submitted(Vec<String>),
    /// Policy requires the hold-to-confirm strip first. Nothing was submitted.
    NeedsConfirmation(Box<Intent>),
    /// The intent was refused.
    Refused(IntentError),
    /// There is no active account to dispatch to.
    NoAccount,
}

/// A store fed by a scenario-playing source.
#[derive(Debug)]
pub struct AccountSession {
    scenario: String,
    source: MockSource,
    store: Store,
    clock_ms: u64,
    paused: bool,
}

impl AccountSession {
    /// Creates a session at offset 0 with nothing applied.
    pub fn new(scenario: &Scenario) -> Self {
        Self {
            scenario: scenario.name.clone(),
            source: MockSource::new(scenario),
            store: Store::new(),
            clock_ms: 0,
            paused: false,
        }
    }

    /// Name of the scenario this session plays.
    pub fn scenario_name(&self) -> &str {
        &self.scenario
    }

    /// The account's store, read-only.
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Offset of the session clock in ms.
    pub fn clock_ms(&self) -> u64 {
        self.clock_ms
    }

    /// Wall-clock time of the session clock.
    pub fn now(&self) -> Timestamp {
        self.source.epoch().saturating_add_ms(self.clock_ms)
    }

    /// Offset of the next scheduled event, if any.
    pub fn next_due(&self) -> Option<u64> {
        self.source.next_due()
    }

    /// Number of effects the source received.
    pub fn submitted_effects(&self) -> usize {
        self.source.effects().len()
    }

    /// Labels of every effect the source received, oldest first.
    pub fn effect_labels(&self) -> impl Iterator<Item = &str> {
        self.source.effects().iter().map(|record| record.label.as_str())
    }

    /// Whether the clock is frozen.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Freezes or unfreezes the clock.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// Moves the clock forward by `ms` and applies due envelopes. Does
    /// nothing while paused.
    pub fn advance(&mut self, ms: u64) -> Applied {
        if self.paused || ms == 0 {
            return Applied::default();
        }
        self.clock_ms = self.clock_ms.saturating_add(ms);
        let mut applied = self.pump();
        applied.changes.mark_clock();
        applied
    }

    /// Applies exactly the next scheduled envelope, paused or not, moving the
    /// clock to it when it lies ahead.
    pub fn step(&mut self) -> Applied {
        let mut applied = Applied::default();
        if let Some(env) = self.source.drain_one() {
            self.clock_ms = self.clock_ms.max(self.source.now_ms());
            self.apply(&env, &mut applied);
            applied.changes.mark_clock();
        }
        applied
    }

    /// Resets the store and replays everything scheduled up to `ms`,
    /// including responses to effects submitted before the seek.
    pub fn seek(&mut self, ms: u64) -> Applied {
        self.source.rewind();
        self.store = Store::new();
        self.clock_ms = ms;
        let mut applied = self.pump();
        let mut changes = ChangeSet::reset();
        changes.mark_clock();
        applied.changes = changes;
        applied
    }

    /// Runs policy for `intent` at the session time and submits the
    /// resulting effects. A confirmation request is handed back unsubmitted.
    pub fn dispatch(&mut self, intent: Intent) -> (DispatchOutcome, Vec<InspectorEntry>) {
        let name = intent_name(&intent);
        let actor = format!("{:?}", intent.actor);
        let mut records = Vec::new();
        let effects = match handle_intent(&self.store, intent, self.now()) {
            Ok(effects) => effects,
            Err(error) => {
                records.push(InspectorEntry::Intent {
                    intent: name,
                    actor,
                    result: format!("refused: {error}"),
                });
                return (DispatchOutcome::Refused(error), records);
            }
        };
        let mut labels = Vec::new();
        for effect in effects {
            if let Effect::RequireConfirmation { intent } = effect {
                records.push(InspectorEntry::Intent {
                    intent: name,
                    actor,
                    result: "confirmation required".to_owned(),
                });
                return (DispatchOutcome::NeedsConfirmation(intent), records);
            }
            let label = effect_label(&effect);
            records.push(InspectorEntry::Effect { label: label.clone() });
            labels.push(label);
            self.source.submit(effect);
        }
        (DispatchOutcome::Submitted(labels), records)
    }

    fn pump(&mut self) -> Applied {
        let mut applied = Applied::default();
        for env in self.source.drain(self.now()) {
            self.apply(&env, &mut applied);
        }
        applied
    }

    fn apply(&mut self, env: &EventEnvelope, applied: &mut Applied) {
        let result = self.store.apply(env);
        applied.records.push(InspectorEntry::Envelope {
            seq: env.seq,
            at_ms: env.at.saturating_since(self.source.epoch()),
            event: event_name(&env.event),
            error: result.as_ref().err().map(ToString::to_string),
        });
        if let Ok(changes) = result {
            applied.changes.merge(changes);
        }
    }
}

#[cfg(test)]
mod tests {
    use ramflux_desktop_core::{Actor, GrantId, GrantState, IntentKind, ParticipantId, methods};
    use ramflux_desktop_test::load_fixture;

    use super::*;

    fn session(name: &str) -> AccountSession {
        AccountSession::new(&load_fixture(name).expect("fixture"))
    }

    fn approve(actor: Actor, confirmed: bool) -> Intent {
        Intent {
            actor,
            kind: IntentKind::Approve { grant_id: GrantId::new("g-rotate"), confirmed },
        }
    }

    #[test]
    fn seek_to_zero_replays_to_the_same_state() {
        let mut live = session("s02_daily");
        live.advance(1_000);
        live.advance(2_000);
        let before = format!("{:?}", live.store());

        let at_zero = load_fixture("s02_daily")
            .expect("fixture")
            .timeline
            .iter()
            .filter(|(offset, _)| *offset == 0)
            .count();
        let applied = live.seek(0);
        assert!(applied.changes.is_reset());
        assert_eq!(live.store().last_seq(), u64::try_from(at_zero).unwrap_or(0), "only t=0 events");
        live.seek(3_000);
        assert_eq!(format!("{:?}", live.store()), before);
    }

    #[test]
    fn paused_clock_does_not_move_but_step_applies() {
        let mut s = session("s04_safety");
        s.set_paused(true);
        assert!(s.advance(10_000).changes.is_empty());
        assert_eq!(s.store().last_seq(), 0);
        assert_eq!(s.step().records.len(), 1);
        assert_eq!(s.store().last_seq(), 1);
    }

    #[test]
    fn high_risk_approval_is_held_for_confirmation_for_humans_and_agents() {
        let mut s = session("s03_intervention");
        s.advance(1_000);
        for actor in [Actor::Human, Actor::Agent(ParticipantId::new("p-orbit"))] {
            let (outcome, _) = s.dispatch(approve(actor.clone(), true));
            match (&actor, outcome) {
                (Actor::Human, DispatchOutcome::Submitted(labels)) => {
                    assert_eq!(labels, [methods::GRANT_APPROVE]);
                }
                (Actor::Agent(_), DispatchOutcome::NeedsConfirmation(intent)) => {
                    assert_eq!(intent.actor, actor);
                }
                (_, other) => panic!("unexpected outcome {other:?}"),
            }
        }
        s.advance(500);
        let grant = s.store().grant(&GrantId::new("g-rotate")).expect("grant");
        assert_eq!(grant.state, GrantState::Approved);
    }
}
